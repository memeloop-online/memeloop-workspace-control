use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::Response,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::plugins::{PluginRequestContext, PluginRoutePermission};

use super::super::{ApiError, AppState, auth::principal};

#[derive(Debug, Deserialize)]
pub(crate) struct PluginApiQuery {
    organization_id: Option<Uuid>,
}

pub(crate) async fn invoke_api_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    method: Method,
    Path((plugin_id, route_id, path)): Path<(String, String, String)>,
    Query(query): Query<PluginApiQuery>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    state.plugins.synchronize().await?;
    let plugin = state
        .plugins
        .plugin_views()?
        .into_iter()
        .find(|item| item.manifest.id == plugin_id && item.enabled)
        .ok_or(crate::plugins::PluginError::NotFound)?;
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
        .find(|route| route.id == route_id)
        .ok_or(crate::plugins::PluginError::NotFound)?;
    if query.organization_id.is_some_and(|id| {
        !actor.system_admin
            && !actor
                .memberships
                .iter()
                .any(|membership| membership.organization_id == id)
    }) {
        return Err(ApiError::Forbidden);
    }
    let permitted = match route.permission {
        PluginRoutePermission::Authenticated => true,
        PluginRoutePermission::OrganizationAdmin => query.organization_id.is_some_and(|id| {
            actor.system_admin || actor.allows(crate::auth::Permission::ManageOrganization, id)
        }),
        PluginRoutePermission::SystemAdmin => actor.system_admin,
    };
    if !permitted {
        return Err(ApiError::Forbidden);
    }
    let response = state
        .plugins
        .invoke_api_route(
            &plugin_id,
            &route_id,
            PluginRequestContext {
                installation_id: state.config.installation_id.to_string(),
                actor_user_id: Some(actor.user_id),
                organization_id: query.organization_id,
                method: method.as_str().to_owned(),
                path,
            },
            body.to_vec(),
        )
        .await?;
    let mut output = Response::new(response.body.into());
    *output.status_mut() = StatusCode::from_u16(response.status)
        .map_err(|_| ApiError::PluginDistribution("plugin_execution_failed"))?;
    output.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&response.content_type)
            .map_err(|_| ApiError::PluginDistribution("plugin_execution_failed"))?,
    );
    output
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    output.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    Ok(output)
}

pub(crate) async fn api_middleware(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_owned();
    if bypass(&path) {
        return next.run(request).await;
    }
    let has_middleware = match state.plugins.has_api_middleware().await {
        Ok(value) => value,
        Err(error) => return ApiError::Plugin(error).into_response(),
    };
    if !has_middleware {
        return next.run(request).await;
    }
    let actor = match optional_actor(&state, request.headers()).await {
        Ok(value) => value,
        Err(error) => return error.into_response(),
    };
    let requested_organization = request.uri().query().and_then(|query| {
        url::form_urlencoded::parse(query.as_bytes()).find_map(|(key, value)| {
            (key == "organization_id")
                .then(|| value.parse().ok())
                .flatten()
        })
    });
    let organization_id = requested_organization.filter(|id| {
        actor.as_ref().is_some_and(|principal| {
            principal.system_admin
                || principal
                    .memberships
                    .iter()
                    .any(|membership| membership.organization_id == *id)
        })
    });
    let context = PluginRequestContext {
        installation_id: state.config.installation_id.to_string(),
        actor_user_id: actor.as_ref().map(|actor| actor.user_id),
        organization_id,
        method: request.method().as_str().to_owned(),
        path,
    };
    match state.plugins.check_api_middleware(context).await {
        Ok(()) => next.run(request).await,
        Err(crate::plugins::PluginError::MiddlewareDenied) => {
            ApiError::PluginDistribution("plugin_middleware_denied").into_response()
        }
        Err(error) => ApiError::Plugin(error).into_response(),
    }
}

fn bypass(path: &str) -> bool {
    !path.starts_with("/api/v1/")
        || path == "/api/v1/openapi.json"
        || path.starts_with("/api/v1/plugins")
        || path.starts_with("/api/v1/plugin-ui")
        || path.starts_with("/api/v1/plugin-api")
        || path.starts_with("/api/v1/internal/")
        || path.starts_with("/api/v1/admin/")
        || path.starts_with("/api/v1/system/")
        || path == "/api/v1/audit"
}

async fn optional_actor(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<crate::storage::Principal>, ApiError> {
    let Some(token) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    else {
        return Ok(None);
    };
    Ok(state.database.authenticate(token).await?)
}

use axum::response::IntoResponse as _;
