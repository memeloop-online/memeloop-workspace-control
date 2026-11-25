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
        IDEMPOTENCY_TTL_SECONDS, hash, idempotency_key, replay_response, unix_timestamp,
    },
    workspace_response::complete_workspace_response,
};
use crate::storage::IdempotencyDecision;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdateWorkspaceSettingsRequest {
    pub name: String,
    pub cpu_millis: u64,
    pub memory_mib: u64,
    pub gpu_count: u32,
    pub expected_generation: u64,
}

#[utoipa::path(
    put,
    path = "/api/v1/workspaces/{workspace_id}/settings",
    request_body = UpdateWorkspaceSettingsRequest,
    params(
        ("workspace_id" = Uuid, Path),
        ("Idempotency-Key" = String, Header, description = "Unique key for this request")
    ),
    responses(
        (status = 200, description = "Settings saved; an eligible memory increase applies asynchronously", body = super::workspaces::WorkspaceResponse),
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
    Json(request): Json<UpdateWorkspaceSettingsRequest>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    let existing = state.database.get_workspace(workspace_id).await?;
    if !actor.allows(
        crate::auth::Permission::ChangeWorkspaceState,
        existing.organization_id,
    ) || !actor.may_access_workspace_template(existing.template_id)
    {
        return Err(ApiError::Forbidden);
    }
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&serde_json::json!({
        "workspace_id": workspace_id,
        "request": &request,
    }))?;
    let scope = format!("{}:workspace-settings-update", actor.user_id);
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
        .update_workspace_settings(
            workspace_id,
            crate::storage::WorkspaceSettingsUpdate {
                name: request.name,
                cpu_millis: request.cpu_millis,
                memory_mib: request.memory_mib,
                gpu_count: request.gpu_count,
                expected_generation: request.expected_generation,
                allow_live_memory_increase: state.workspace_live_memory_resize_supported(),
                actor_user_id: actor.user_id,
                now,
            },
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
    let expose_connection = actor.allows(
        crate::auth::Permission::ConnectWorkspace,
        workspace.organization_id,
    );
    complete_workspace_response(
        &state,
        &scope,
        key,
        &request_hash,
        StatusCode::OK,
        workspace,
        expose_connection,
    )
    .await
}
