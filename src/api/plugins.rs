use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    auth::Permission,
    storage::{IdempotencyDecision, PluginConfigurationWrite, Principal},
};

use super::{
    ApiError, AppState,
    auth::principal,
    idempotency::{
        IDEMPOTENCY_TTL_SECONDS, hash, idempotency_key, json_response, replay_response,
        unix_timestamp,
    },
};

mod backend;
pub(super) mod lifecycle;
mod surfaces;

pub(super) use backend::{api_middleware, invoke_api_route};
pub(super) use lifecycle::{
    confirm_install, inspect_github_release, inspect_upload, inspect_url, list_packages,
    set_enabled, uninstall,
};
pub(super) use surfaces::{bridge, create_surface_session, surface_asset};

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct PluginManifestView {
    id: String,
    name: String,
    version: String,
    description: String,
    wit_version: String,
    workspace_create_policy: bool,
    denial_codes: Vec<String>,
    declared_contributions: Vec<String>,
    approved_contributions: Vec<String>,
    configuration_schema: Option<Value>,
    configuration_default: Option<Value>,
    loaded: bool,
    source: &'static str,
    error_code: Option<&'static str>,
    error_message: Option<&'static str>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub(super) struct PluginConfigurationQuery {
    organization_id: Option<Uuid>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct PutPluginConfigurationRequest {
    expected_version: u64,
    value: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct DeletePluginConfigurationRequest {
    expected_version: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct PluginConfigurationView {
    plugin_id: String,
    scope: &'static str,
    organization_id: Option<Uuid>,
    source: &'static str,
    scope_version: u64,
    effective_version: u64,
    value: Value,
    schema_digest: String,
    stored_schema_digest: Option<String>,
    schema_changed: bool,
    valid: bool,
    updated_at: Option<i64>,
}

#[utoipa::path(get, path = "/api/v1/plugins/{plugin_id}/configuration", params(("plugin_id" = String, Path), PluginConfigurationQuery), responses((status = 200, body = PluginConfigurationView), (status = 403, body = super::ErrorEnvelope), (status = 404, body = super::ErrorEnvelope)))]
pub(super) async fn get_configuration(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(plugin_id): Path<String>,
    Query(query): Query<PluginConfigurationQuery>,
) -> Result<Json<PluginConfigurationView>, ApiError> {
    let actor = principal(&state, &headers).await?;
    authorize_scope(&actor, query.organization_id)?;
    Ok(Json(
        configuration_view(&state, &plugin_id, query.organization_id).await?,
    ))
}

#[utoipa::path(put, path = "/api/v1/plugins/{plugin_id}/configuration", request_body = PutPluginConfigurationRequest, params(("plugin_id" = String, Path), PluginConfigurationQuery, ("Idempotency-Key" = String, Header)), responses((status = 200, body = PluginConfigurationView), (status = 409, body = super::ErrorEnvelope), (status = 422, body = super::ErrorEnvelope)))]
pub(super) async fn put_configuration(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(plugin_id): Path<String>,
    Query(query): Query<PluginConfigurationQuery>,
    Json(request): Json<PutPluginConfigurationRequest>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    authorize_scope(&actor, query.organization_id)?;
    state
        .plugins
        .validate_configuration(&plugin_id, &request.value)?;
    let schema_digest = state.plugins.configuration_schema_digest(&plugin_id)?;
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&(&plugin_id, query.organization_id, &request))?;
    let scope = operation_scope(actor.user_id, &plugin_id, query.organization_id, "put");
    let now = unix_timestamp()?;
    if let Some(response) = reserve(&state, &scope, key, &request_hash, now).await? {
        return Ok(response);
    }
    let result = state
        .database
        .put_plugin_configuration(PluginConfigurationWrite {
            plugin_id: &plugin_id,
            organization_id: query.organization_id,
            value: &request.value,
            schema_digest: &schema_digest,
            expected_version: request.expected_version,
            actor_user_id: actor.user_id,
            now,
        })
        .await;
    if let Err(error) = result {
        state
            .database
            .abandon_idempotency(&scope, key, &request_hash)
            .await?;
        return Err(error.into());
    }
    state
        .database
        .record_audit(
            Some(actor.user_id),
            query.organization_id,
            None,
            "plugin.configuration.put",
            serde_json::json!({"plugin_id": plugin_id, "scope": scope_name(query.organization_id), "version": request.expected_version.saturating_add(1)}),
            now,
        )
        .await?;
    let view = configuration_view(&state, &plugin_id, query.organization_id).await?;
    finish(&state, &scope, key, &request_hash, &view).await
}

#[utoipa::path(delete, path = "/api/v1/plugins/{plugin_id}/configuration", request_body = DeletePluginConfigurationRequest, params(("plugin_id" = String, Path), PluginConfigurationQuery, ("Idempotency-Key" = String, Header)), responses((status = 200, body = PluginConfigurationView), (status = 409, body = super::ErrorEnvelope)))]
pub(super) async fn delete_configuration(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(plugin_id): Path<String>,
    Query(query): Query<PluginConfigurationQuery>,
    Json(request): Json<DeletePluginConfigurationRequest>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    authorize_scope(&actor, query.organization_id)?;
    state.plugins.configuration_schema_digest(&plugin_id)?;
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&(&plugin_id, query.organization_id, &request))?;
    let scope = operation_scope(actor.user_id, &plugin_id, query.organization_id, "delete");
    let now = unix_timestamp()?;
    if let Some(response) = reserve(&state, &scope, key, &request_hash, now).await? {
        return Ok(response);
    }
    if let Err(error) = state
        .database
        .delete_plugin_configuration(&plugin_id, query.organization_id, request.expected_version)
        .await
    {
        state
            .database
            .abandon_idempotency(&scope, key, &request_hash)
            .await?;
        return Err(error.into());
    }
    state
        .database
        .record_audit(
            Some(actor.user_id),
            query.organization_id,
            None,
            "plugin.configuration.delete",
            serde_json::json!({"plugin_id": plugin_id, "scope": scope_name(query.organization_id), "deleted_version": request.expected_version}),
            now,
        )
        .await?;
    let view = configuration_view(&state, &plugin_id, query.organization_id).await?;
    finish(&state, &scope, key, &request_hash, &view).await
}

async fn configuration_view(
    state: &AppState,
    plugin_id: &str,
    organization_id: Option<Uuid>,
) -> Result<PluginConfigurationView, ApiError> {
    let manifest = state
        .plugins
        .manifest(plugin_id)
        .ok_or(crate::plugins::PluginError::NotFound)?;
    let default = manifest
        .configuration
        .ok_or(crate::plugins::PluginError::InvalidConfiguration)?
        .default;
    let direct = state
        .database
        .plugin_configuration_for_scope(plugin_id, organization_id)
        .await?;
    let inherited = if organization_id.is_some() && direct.is_none() {
        state
            .database
            .plugin_configuration_for_scope(plugin_id, None)
            .await?
    } else {
        None
    };
    let effective = direct.as_ref().or(inherited.as_ref());
    let value = effective
        .map(|stored| stored.value.clone())
        .unwrap_or(default);
    let source = if direct.is_some() {
        scope_name(organization_id)
    } else if inherited.is_some() {
        "installation"
    } else {
        "default"
    };
    let digest = state.plugins.configuration_schema_digest(plugin_id)?;
    let stored_digest = effective.map(|stored| stored.schema_digest.clone());
    let valid = state
        .plugins
        .validate_configuration(plugin_id, &value)
        .is_ok();
    Ok(PluginConfigurationView {
        plugin_id: plugin_id.to_owned(),
        scope: scope_name(organization_id),
        organization_id,
        source,
        scope_version: direct.as_ref().map_or(0, |stored| stored.version),
        effective_version: effective.map_or(0, |stored| stored.version),
        value,
        schema_changed: stored_digest
            .as_deref()
            .is_some_and(|stored| stored != digest),
        schema_digest: digest,
        stored_schema_digest: stored_digest,
        valid,
        updated_at: effective.map(|stored| stored.updated_at),
    })
}

fn authorize_scope(actor: &Principal, organization_id: Option<Uuid>) -> Result<(), ApiError> {
    let allowed = organization_id.map_or(actor.system_admin, |organization_id| {
        actor.system_admin || actor.allows(Permission::ManageOrganization, organization_id)
    });
    if !allowed {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

fn scope_name(organization_id: Option<Uuid>) -> &'static str {
    if organization_id.is_some() {
        "organization"
    } else {
        "installation"
    }
}

fn operation_scope(
    user_id: Uuid,
    plugin_id: &str,
    organization_id: Option<Uuid>,
    action: &str,
) -> String {
    let selected = organization_id.map_or_else(|| "installation".to_owned(), |id| id.to_string());
    format!("{user_id}:plugin-{action}:{plugin_id}:{selected}")
}

async fn reserve(
    state: &AppState,
    scope: &str,
    key: &str,
    request_hash: &str,
    now: i64,
) -> Result<Option<Response>, ApiError> {
    Ok(
        match state
            .database
            .begin_idempotency(scope, key, request_hash, now, now + IDEMPOTENCY_TTL_SECONDS)
            .await?
        {
            IdempotencyDecision::Replay(replay) => Some(replay_response(replay)?),
            IdempotencyDecision::Conflict => return Err(ApiError::IdempotencyConflict),
            IdempotencyDecision::InProgress => return Err(ApiError::IdempotencyInProgress),
            IdempotencyDecision::Reserved => None,
        },
    )
}

async fn finish(
    state: &AppState,
    scope: &str,
    key: &str,
    request_hash: &str,
    view: &PluginConfigurationView,
) -> Result<Response, ApiError> {
    let body = serde_json::to_string(view)
        .map_err(|_| ApiError::BadRequest("plugin response could not be encoded"))?;
    state
        .database
        .finish_idempotency(scope, key, request_hash, StatusCode::OK.as_u16(), &body)
        .await?;
    json_response(StatusCode::OK, body)
}
