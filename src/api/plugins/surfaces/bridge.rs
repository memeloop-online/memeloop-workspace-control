use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    auth::Permission,
    plugins::{PluginRequestContext, PluginRoutePermission},
    storage::Principal,
};

use super::{
    super::super::{ApiError, AppState, auth::principal, idempotency::unix_timestamp},
    helpers::{
        cookie_value, digest, safe_method, safe_path, safe_route_id, validate_session_package,
    },
};

const MAX_BRIDGE_BODY_BYTES: usize = 256 * 1024;

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PluginBridgeRequest {
    request_id: String,
    method: String,
    payload: Value,
    channel_nonce: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct PluginBridgeResponse {
    request_id: String,
    result: Option<Value>,
    error: Option<PluginBridgeError>,
}

#[derive(Debug, Serialize, ToSchema)]
struct PluginBridgeError {
    code: &'static str,
    message: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginApiBridgePayload {
    route_id: String,
    method: String,
    path: String,
    body: Option<Value>,
    organization_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PluginBridgeQuery {
    organization_id: Option<Uuid>,
}

pub(crate) async fn bridge(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((plugin_id, session_id)): Path<(String, Uuid)>,
    Query(query): Query<PluginBridgeQuery>,
    Json(request): Json<PluginBridgeRequest>,
) -> Result<Json<PluginBridgeResponse>, ApiError> {
    let actor = principal(&state, &headers).await?;
    let cookie = cookie_value(&headers, session_id)
        .ok_or(ApiError::PluginDistribution("plugin_ui_session_invalid"))?;
    let session = state
        .database
        .plugin_ui_session_by_cookie(session_id, &digest(cookie), unix_timestamp()?)
        .await?;
    validate_session_package(&state, &session, &plugin_id).await?;
    if session.user_id != actor.user_id
        || session.channel_nonce != request.channel_nonce
        || !session.allowed_bridge_methods.contains(&request.method)
    {
        return Err(ApiError::Forbidden);
    }
    match request.method.as_str() {
        "theme.read" => Ok(Json(PluginBridgeResponse {
            request_id: request.request_id,
            result: Some(serde_json::json!({"theme":"system"})),
            error: None,
        })),
        "plugin_api.request" => invoke_plugin_api(
            &state,
            &actor,
            &plugin_id,
            query.organization_id,
            request.request_id,
            request.payload,
        )
        .await
        .map(Json),
        _ => Err(ApiError::PluginDistribution(
            "plugin_bridge_request_invalid",
        )),
    }
}

async fn invoke_plugin_api(
    state: &AppState,
    actor: &Principal,
    plugin_id: &str,
    query_organization_id: Option<Uuid>,
    request_id: String,
    value: Value,
) -> Result<PluginBridgeResponse, ApiError> {
    let payload: PluginApiBridgePayload = serde_json::from_value(value)
        .map_err(|_| ApiError::PluginDistribution("plugin_bridge_request_invalid"))?;
    validate_payload(&payload)?;
    state.plugins.synchronize().await?;
    let plugin = state
        .plugins
        .plugin_views()?
        .into_iter()
        .find(|item| item.manifest.id == plugin_id && item.enabled)
        .ok_or(ApiError::PluginDistribution("plugin_not_found"))?;
    if !plugin
        .approved_contributions
        .iter()
        .any(|value| value == "api_routes")
    {
        return Err(ApiError::PluginDistribution(
            "plugin_capability_not_approved",
        ));
    }
    let route = plugin
        .manifest
        .api_routes
        .iter()
        .find(|route| route.id == payload.route_id)
        .ok_or(ApiError::PluginDistribution("plugin_route_not_found"))?;
    let organization_id = payload.organization_id.or(query_organization_id);
    authorize_route(actor, route.permission, organization_id)?;
    let body = payload.body.map_or_else(Vec::new, |body| {
        serde_json::to_vec(&body).unwrap_or_default()
    });
    if body.len() > MAX_BRIDGE_BODY_BYTES {
        return Err(ApiError::PluginDistribution(
            "plugin_bridge_request_invalid",
        ));
    }
    let output = state
        .plugins
        .invoke_api_route(
            plugin_id,
            &payload.route_id,
            PluginRequestContext {
                installation_id: state.config.installation_id.to_string(),
                actor_user_id: Some(actor.user_id),
                organization_id,
                method: payload.method,
                path: payload.path,
            },
            body,
        )
        .await?;
    let body = if output.content_type == "application/json" {
        serde_json::from_slice(&output.body)
            .map_err(|_| ApiError::PluginDistribution("plugin_execution_failed"))?
    } else {
        Value::String(
            String::from_utf8(output.body)
                .map_err(|_| ApiError::PluginDistribution("plugin_execution_failed"))?,
        )
    };
    Ok(PluginBridgeResponse {
        request_id,
        result: Some(serde_json::json!({
            "status": output.status,
            "content_type": output.content_type,
            "body": body,
        })),
        error: None,
    })
}

fn validate_payload(payload: &PluginApiBridgePayload) -> Result<(), ApiError> {
    if safe_route_id(&payload.route_id) && safe_method(&payload.method) && safe_path(&payload.path)
    {
        Ok(())
    } else {
        Err(ApiError::PluginDistribution(
            "plugin_bridge_request_invalid",
        ))
    }
}

fn authorize_route(
    actor: &Principal,
    permission: PluginRoutePermission,
    organization_id: Option<Uuid>,
) -> Result<(), ApiError> {
    if organization_id.is_some_and(|id| {
        !actor.system_admin
            && !actor
                .memberships
                .iter()
                .any(|membership| membership.organization_id == id)
    }) {
        return Err(ApiError::Forbidden);
    }
    let permitted = match permission {
        PluginRoutePermission::Authenticated => true,
        PluginRoutePermission::OrganizationAdmin => organization_id.is_some_and(|id| {
            actor.system_admin || actor.allows(Permission::ManageOrganization, id)
        }),
        PluginRoutePermission::SystemAdmin => actor.system_admin,
    };
    permitted.then_some(()).ok_or(ApiError::Forbidden)
}
