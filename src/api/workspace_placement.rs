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

use crate::storage::IdempotencyDecision;

use super::{
    ApiError, AppState,
    auth::principal,
    idempotency::{
        IDEMPOTENCY_TTL_SECONDS, hash, idempotency_key, replay_response, unix_timestamp,
    },
    workspace_response::complete_workspace_response,
};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdateWorkspacePlacementRequest {
    pub node_pool: String,
    pub expected_generation: u64,
}

#[utoipa::path(
    put,
    path = "/api/v1/workspaces/{workspace_id}/placement",
    request_body = UpdateWorkspacePlacementRequest,
    params(
        ("workspace_id" = Uuid, Path),
        ("Idempotency-Key" = String, Header)
    ),
    responses(
        (status = 200, body = super::workspaces::WorkspaceResponse),
        (status = 400, body = super::ErrorEnvelope),
        (status = 401, body = super::ErrorEnvelope),
        (status = 403, body = super::ErrorEnvelope),
        (status = 409, body = super::ErrorEnvelope),
        (status = 422, body = super::ErrorEnvelope)
    )
)]
pub(super) async fn update(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(workspace_id): Path<Uuid>,
    Json(request): Json<UpdateWorkspacePlacementRequest>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    let existing = state.database.get_workspace(workspace_id).await?;
    if !actor.may_manage_system()
        || actor.has_template_restriction()
        || !actor.may_access_workspace_template(existing.template_id)
    {
        return Err(ApiError::Forbidden);
    }
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&(workspace_id, &request))?;
    let scope = format!("{}:workspace-placement-update", actor.user_id);
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
    let workspace = match state
        .database
        .update_stopped_workspace_placement(
            workspace_id,
            &request.node_pool,
            request.expected_generation,
            actor.user_id,
            now,
        )
        .await
    {
        Ok(workspace) => workspace,
        Err(error) => {
            state
                .database
                .abandon_idempotency(&scope, key, &request_hash)
                .await?;
            return Err(error.into());
        }
    };
    complete_workspace_response(
        &state,
        &scope,
        key,
        &request_hash,
        StatusCode::OK,
        workspace,
        true,
    )
    .await
}
