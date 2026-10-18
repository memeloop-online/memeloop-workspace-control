use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{
    ApiError, AppState,
    auth::principal,
    idempotency::{
        IDEMPOTENCY_TTL_SECONDS, hash, idempotency_key, json_response, replay_response,
        unix_timestamp,
    },
};
use crate::{
    auth::{Permission, Role},
    storage::{IdempotencyDecision, Principal, StorageShrinkOperation, StorageShrinkRequest},
    workspaces::Workspace,
};

fn authorize(actor: &Principal, workspace: &Workspace, write: bool) -> Result<(), ApiError> {
    let permission = if write {
        Permission::ChangeWorkspaceState
    } else {
        Permission::ReadWorkspace
    };
    let administrator = actor.system_admin
        || actor.memberships.iter().any(|membership| {
            membership.organization_id == workspace.organization_id
                && membership.role == Role::OrganizationAdmin
        });
    if !actor.allows(permission, workspace.organization_id)
        || !actor.may_access_workspace_template(workspace.template_id)
        || actor.user_id != workspace.owner_id && !administrator
    {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ConfirmStorageShrinkRequest {
    pub operation_id: Uuid,
}

#[utoipa::path(
    post,
    path = "/api/v1/workspaces/{workspace_id}/storage-shrink/retry",
    request_body = ConfirmStorageShrinkRequest,
    params(("workspace_id" = Uuid, Path), ("Idempotency-Key" = String, Header)),
    responses((status = 202, body = StorageShrinkOperation), (status = 401, body = super::ErrorEnvelope), (status = 403, body = super::ErrorEnvelope), (status = 404, body = super::ErrorEnvelope), (status = 409, body = super::ErrorEnvelope), (status = 503, body = super::ErrorEnvelope))
)]
pub(super) async fn retry(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(workspace_id): Path<Uuid>,
    Json(request): Json<ConfirmStorageShrinkRequest>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    let workspace = state.database.get_workspace(workspace_id).await?;
    authorize(&actor, &workspace, true)?;
    if state.kubernetes_client.is_none() {
        return Err(ApiError::KubernetesUnavailable);
    }
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&(workspace_id, &request))?;
    let scope = format!("{}:workspace-storage-shrink-retry", actor.user_id);
    let now = unix_timestamp()?;
    match state
        .database
        .begin_idempotency(
            &scope,
            key,
            &request_hash,
            now,
            now + IDEMPOTENCY_TTL_SECONDS,
        )
        .await?
    {
        IdempotencyDecision::Replay(replay) => return replay_response(replay),
        IdempotencyDecision::Conflict => return Err(ApiError::IdempotencyConflict),
        IdempotencyDecision::InProgress => return Err(ApiError::IdempotencyInProgress),
        IdempotencyDecision::Reserved => {}
    }
    let operation = match state
        .database
        .retry_storage_shrink_resume(workspace_id, request.operation_id, actor.user_id, now)
        .await
    {
        Ok(operation) => operation,
        Err(error) => {
            state
                .database
                .abandon_idempotency(&scope, key, &request_hash)
                .await?;
            return Err(error.into());
        }
    };
    let json = serde_json::to_string(&operation)
        .map_err(|_| ApiError::BadRequest("response serialization failed"))?;
    state
        .database
        .finish_idempotency(
            &scope,
            key,
            &request_hash,
            StatusCode::ACCEPTED.as_u16(),
            &json,
        )
        .await?;
    json_response(StatusCode::ACCEPTED, json)
}

#[utoipa::path(
    post,
    path = "/api/v1/workspaces/{workspace_id}/storage-shrink/confirm",
    request_body = ConfirmStorageShrinkRequest,
    params(("workspace_id" = Uuid, Path), ("Idempotency-Key" = String, Header)),
    responses((status = 202, body = StorageShrinkOperation), (status = 401, body = super::ErrorEnvelope), (status = 403, body = super::ErrorEnvelope), (status = 404, body = super::ErrorEnvelope), (status = 409, body = super::ErrorEnvelope))
)]
pub(super) async fn confirm(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(workspace_id): Path<Uuid>,
    Json(request): Json<ConfirmStorageShrinkRequest>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    let workspace = state.database.get_workspace(workspace_id).await?;
    authorize(&actor, &workspace, true)?;
    if state.kubernetes_client.is_none() {
        return Err(ApiError::KubernetesUnavailable);
    }
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&(workspace_id, &request))?;
    let scope = format!("{}:workspace-storage-shrink-confirm", actor.user_id);
    let now = unix_timestamp()?;
    match state
        .database
        .begin_idempotency(
            &scope,
            key,
            &request_hash,
            now,
            now + IDEMPOTENCY_TTL_SECONDS,
        )
        .await?
    {
        IdempotencyDecision::Replay(replay) => return replay_response(replay),
        IdempotencyDecision::Conflict => return Err(ApiError::IdempotencyConflict),
        IdempotencyDecision::InProgress => return Err(ApiError::IdempotencyInProgress),
        IdempotencyDecision::Reserved => {}
    }
    let operation = match state
        .database
        .confirm_storage_shrink(workspace_id, request.operation_id, actor.user_id, now)
        .await
    {
        Ok(operation) => operation,
        Err(error) => {
            state
                .database
                .abandon_idempotency(&scope, key, &request_hash)
                .await?;
            return Err(error.into());
        }
    };
    let json = serde_json::to_string(&operation)
        .map_err(|_| ApiError::BadRequest("response serialization failed"))?;
    state
        .database
        .finish_idempotency(
            &scope,
            key,
            &request_hash,
            StatusCode::ACCEPTED.as_u16(),
            &json,
        )
        .await?;
    json_response(StatusCode::ACCEPTED, json)
}

#[utoipa::path(
    post,
    path = "/api/v1/workspaces/{workspace_id}/storage-shrink",
    request_body = StorageShrinkRequest,
    params(("workspace_id" = Uuid, Path), ("Idempotency-Key" = String, Header)),
    responses((status = 202, body = StorageShrinkOperation), (status = 400, body = super::ErrorEnvelope), (status = 401, body = super::ErrorEnvelope), (status = 403, body = super::ErrorEnvelope), (status = 404, body = super::ErrorEnvelope), (status = 409, body = super::ErrorEnvelope), (status = 422, body = super::ErrorEnvelope), (status = 503, body = super::ErrorEnvelope))
)]
pub(super) async fn create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(workspace_id): Path<Uuid>,
    Json(request): Json<StorageShrinkRequest>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    let workspace = state.database.get_workspace(workspace_id).await?;
    authorize(&actor, &workspace, true)?;
    if state.kubernetes_client.is_none() {
        return Err(ApiError::KubernetesUnavailable);
    }
    crate::kubernetes::storage_shrink_image()?;
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&(workspace_id, &request))?;
    let scope = format!("{}:workspace-storage-shrink", actor.user_id);
    let now = unix_timestamp()?;
    match state
        .database
        .begin_idempotency(
            &scope,
            key,
            &request_hash,
            now,
            now + IDEMPOTENCY_TTL_SECONDS,
        )
        .await?
    {
        IdempotencyDecision::Replay(replay) => return replay_response(replay),
        IdempotencyDecision::Conflict => return Err(ApiError::IdempotencyConflict),
        IdempotencyDecision::InProgress => return Err(ApiError::IdempotencyInProgress),
        IdempotencyDecision::Reserved => {}
    }
    let operation = match state
        .database
        .enqueue_storage_shrink(workspace_id, request, actor.user_id, now)
        .await
    {
        Ok(operation) => operation,
        Err(error) => {
            state
                .database
                .abandon_idempotency(&scope, key, &request_hash)
                .await?;
            return Err(error.into());
        }
    };
    let json = serde_json::to_string(&operation)
        .map_err(|_| ApiError::BadRequest("response serialization failed"))?;
    state
        .database
        .finish_idempotency(
            &scope,
            key,
            &request_hash,
            StatusCode::ACCEPTED.as_u16(),
            &json,
        )
        .await?;
    json_response(StatusCode::ACCEPTED, json)
}

#[utoipa::path(
    get,
    path = "/api/v1/workspaces/{workspace_id}/storage-shrink",
    params(("workspace_id" = Uuid, Path)),
    responses((status = 200, body = Option<StorageShrinkOperation>), (status = 401, body = super::ErrorEnvelope), (status = 403, body = super::ErrorEnvelope), (status = 404, body = super::ErrorEnvelope))
)]
pub(super) async fn get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<Option<StorageShrinkOperation>>, ApiError> {
    let actor = principal(&state, &headers).await?;
    let workspace = state.database.get_workspace(workspace_id).await?;
    authorize(&actor, &workspace, false)?;
    Ok(Json(
        state.database.latest_storage_shrink(workspace_id).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_rejects_unknown_fields() {
        let operation_id = Uuid::now_v7();
        assert!(
            serde_json::from_value::<ConfirmStorageShrinkRequest>(serde_json::json!({
                "operation_id": operation_id,
                "reclaim_source": true,
            }))
            .is_err()
        );
        assert!(serde_json::from_str::<ConfirmStorageShrinkRequest>("{}").is_err());
        assert!(
            serde_json::from_str::<StorageShrinkRequest>(
                r#"{"home_gib":20,"expected_generation":1,"delete_source":true}"#
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<StorageShrinkRequest>(
                r#"{"home_gib":20,"expected_generation":1}"#
            )
            .is_ok()
        );
    }
}
