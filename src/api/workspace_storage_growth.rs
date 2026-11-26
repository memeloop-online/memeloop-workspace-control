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
    kubernetes::storage_growth::{GrowthError, GrowthVolume, select_claim},
    storage::{
        IdempotencyDecision, StorageError,
        workspace_storage_growth::{StorageGrowth, StorageGrowthStatus},
    },
    workspaces::WorkspaceState,
};

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct StorageGrowthRequest {
    volume: GrowthVolume,
    capacity_gib: u64,
    expected_generation: u64,
}

#[utoipa::path(
    post,
    path = "/api/v1/workspaces/{workspace_id}/storage-growth",
    request_body = StorageGrowthRequest,
    params(("workspace_id" = Uuid, Path), ("Idempotency-Key" = String, Header)),
    responses(
        (status = 202, body = StorageGrowthStatus),
        (status = 403, body = super::ErrorEnvelope),
        (status = 409, body = super::ErrorEnvelope),
        (status = 422, body = super::ErrorEnvelope),
        (status = 502, body = super::ErrorEnvelope)
    )
)]
pub(super) async fn create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(request): Json<StorageGrowthRequest>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.may_manage_system() || actor.has_template_restriction() {
        return Err(ApiError::Forbidden);
    }
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&(id, &request))?;
    let scope = format!("{}:workspace-storage-growth", actor.user_id);
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
    let result = enqueue(&state, id, actor.user_id, &request, now).await;
    let status = match result {
        Ok(status) => status,
        Err(error) => {
            state
                .database
                .abandon_idempotency(&scope, key, &request_hash)
                .await?;
            return Err(error);
        }
    };
    let json = serde_json::to_string(&status)
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

async fn enqueue(
    state: &AppState,
    id: Uuid,
    actor: Uuid,
    request: &StorageGrowthRequest,
    now: i64,
) -> Result<StorageGrowthStatus, ApiError> {
    let workspace = state.database.get_workspace(id).await?;
    if workspace.state != WorkspaceState::Ready
        || workspace.generation != request.expected_generation
    {
        return Err(StorageError::WorkspaceStorageGrowthConflict.into());
    }
    let previous_gib = request.volume.capacity(&workspace);
    let max = match request.volume {
        GrowthVolume::Home => (i64::MAX as u64) >> 30,
        GrowthVolume::TemporaryStorage => 2048,
    };
    if request.capacity_gib <= previous_gib || request.capacity_gib > max {
        return Err(StorageError::InvalidWorkspaceStorageGrowth.into());
    }
    let client = state
        .kubernetes_client
        .as_ref()
        .ok_or(ApiError::KubernetesUnavailable)?;
    let claim = select_claim(
        client,
        &state.config.installation_id,
        &workspace,
        request.volume,
    )
    .await
    .map_err(|error| match error {
        GrowthError::Unsupported => ApiError::from(StorageError::WorkspaceStorageGrowthUnavailable),
        GrowthError::Kubernetes(error) => ApiError::Kubernetes(error),
        GrowthError::CapacityChanged | GrowthError::ClaimChanged | GrowthError::Rejected => {
            ApiError::from(StorageError::WorkspaceStorageGrowthConflict)
        }
    })?;
    let growth = StorageGrowth {
        volume: request.volume,
        claim,
        previous_gib,
        requested_gib: request.capacity_gib,
        rejection: None,
    };
    Ok(state
        .database
        .enqueue_storage_growth(id, request.expected_generation, actor, growth, now)
        .await?)
}

#[utoipa::path(
    get,
    path = "/api/v1/workspaces/{workspace_id}/storage-growth",
    params(("workspace_id" = Uuid, Path)),
    responses((status = 200, body = Option<StorageGrowthStatus>), (status = 403, body = super::ErrorEnvelope))
)]
pub(super) async fn get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Option<StorageGrowthStatus>>, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.may_manage_system() || actor.has_template_restriction() {
        return Err(ApiError::Forbidden);
    }
    state.database.get_workspace(id).await?;
    Ok(Json(state.database.latest_storage_growth(id).await?))
}
