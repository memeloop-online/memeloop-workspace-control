use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    auth::Permission,
    storage::{CreateWorkspaceTemplate, IdempotencyDecision, ImagePolicy, WorkspaceTemplate},
};

use super::{
    ApiError, AppState,
    auth::principal,
    idempotency::{
        IDEMPOTENCY_TTL_SECONDS, hash, idempotency_key, json_response, replay_response,
        unix_timestamp,
    },
};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(super) struct PutImageRequest {
    image: String,
    #[serde(default = "enabled_by_default")]
    enabled: bool,
}

#[derive(Debug, Deserialize, IntoParams)]
pub(super) struct TemplateQuery {
    organization_id: Option<Uuid>,
}

#[utoipa::path(get, path = "/api/v1/admin/images", responses((status = 200, body = [ImagePolicy]), (status = 403, body = super::ErrorEnvelope)))]
pub(super) async fn list_images(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<ImagePolicy>>, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.system_admin {
        return Err(ApiError::Forbidden);
    }
    Ok(Json(state.database.list_image_policies().await?))
}

#[utoipa::path(put, path = "/api/v1/admin/images", request_body = PutImageRequest, params(("Idempotency-Key" = String, Header)), responses((status = 200, body = ImagePolicy), (status = 403, body = super::ErrorEnvelope)))]
pub(super) async fn put_image(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<PutImageRequest>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.system_admin {
        return Err(ApiError::Forbidden);
    }
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&request)?;
    let scope = format!("{}:put-image-policy", actor.user_id);
    let now = unix_timestamp()?;
    if let Some(response) = reserve(&state, &scope, key, &request_hash, now).await? {
        return Ok(response);
    }
    let policy = match state
        .database
        .upsert_image_policy(&request.image, request.enabled, now)
        .await
    {
        Ok(policy) => policy,
        Err(error) => {
            state
                .database
                .abandon_idempotency(&scope, key, &request_hash)
                .await?;
            return Err(error.into());
        }
    };
    state.database.record_audit(Some(actor.user_id), None, None, "image_policy.upsert", serde_json::json!({"image": policy.image, "enabled": policy.enabled, "contract_version": policy.contract_version}), now).await?;
    finish(&state, &scope, key, &request_hash, StatusCode::OK, &policy).await
}

#[utoipa::path(get, path = "/api/v1/templates", params(TemplateQuery), responses((status = 200, body = [WorkspaceTemplate]), (status = 403, body = super::ErrorEnvelope)))]
pub(super) async fn list_templates(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<TemplateQuery>,
) -> Result<Json<Vec<WorkspaceTemplate>>, ApiError> {
    let actor = principal(&state, &headers).await?;
    if let Some(organization_id) = query.organization_id
        && !actor.allows(Permission::ReadWorkspace, organization_id)
    {
        return Err(ApiError::Forbidden);
    }
    Ok(Json(
        state
            .database
            .list_workspace_templates(query.organization_id)
            .await?,
    ))
}

#[utoipa::path(post, path = "/api/v1/templates", request_body = CreateWorkspaceTemplate, params(("Idempotency-Key" = String, Header)), responses((status = 201, body = WorkspaceTemplate), (status = 403, body = super::ErrorEnvelope)))]
pub(super) async fn create_template(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(command): Json<CreateWorkspaceTemplate>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    let allowed = command
        .organization_id
        .map_or(actor.system_admin, |organization_id| {
            actor.allows(Permission::ManageOrganization, organization_id)
        });
    if !allowed {
        return Err(ApiError::Forbidden);
    }
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&command)?;
    let scope = format!("{}:create-template", actor.user_id);
    let now = unix_timestamp()?;
    if let Some(response) = reserve(&state, &scope, key, &request_hash, now).await? {
        return Ok(response);
    }
    let template = match state.database.create_workspace_template(command, now).await {
        Ok(template) => template,
        Err(error) => {
            state
                .database
                .abandon_idempotency(&scope, key, &request_hash)
                .await?;
            return Err(error.into());
        }
    };
    state.database.record_audit(Some(actor.user_id), template.organization_id, None, "template.create", serde_json::json!({"template_id": template.id, "name": template.name, "image": template.image}), now).await?;
    finish(
        &state,
        &scope,
        key,
        &request_hash,
        StatusCode::CREATED,
        &template,
    )
    .await
}

async fn reserve(
    state: &AppState,
    scope: &str,
    key: &str,
    request_hash: &str,
    now: i64,
) -> Result<Option<Response>, ApiError> {
    match state
        .database
        .begin_idempotency(scope, key, request_hash, now, now + IDEMPOTENCY_TTL_SECONDS)
        .await?
    {
        IdempotencyDecision::Reserved => Ok(None),
        IdempotencyDecision::Replay(replay) => replay_response(replay).map(Some),
        IdempotencyDecision::Conflict => Err(ApiError::IdempotencyConflict),
        IdempotencyDecision::InProgress => Err(ApiError::IdempotencyInProgress),
    }
}

async fn finish<T: Serialize>(
    state: &AppState,
    scope: &str,
    key: &str,
    request_hash: &str,
    status: StatusCode,
    value: &T,
) -> Result<Response, ApiError> {
    let body = serde_json::to_string(value)
        .map_err(|_| ApiError::BadRequest("response serialization failed"))?;
    state
        .database
        .finish_idempotency(scope, key, request_hash, status.as_u16(), &body)
        .await?;
    json_response(status, body)
}

fn enabled_by_default() -> bool {
    true
}
