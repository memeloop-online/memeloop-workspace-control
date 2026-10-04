use std::{sync::Arc, time::Duration};

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    storage::{IdempotencyDecision, StorageError},
    workspaces::{Workspace, WorkspaceHomeVolumeBinding, WorkspaceState},
};

use super::{
    ApiError, AppState,
    auth::principal,
    idempotency::{
        IDEMPOTENCY_TTL_SECONDS, hash, idempotency_key, replay_response, unix_timestamp,
    },
    workspace_response::complete_workspace_response,
};

mod validation;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct CorrectWorkspaceHomeCapacityRequest {
    pub claim_uid: String,
    pub capacity_gib: u64,
    pub expected_generation: u64,
}

#[utoipa::path(
    put,
    path = "/api/v1/workspaces/{workspace_id}/home-capacity",
    request_body = CorrectWorkspaceHomeCapacityRequest,
    params(("workspace_id" = Uuid, Path), ("Idempotency-Key" = String, Header)),
    responses(
        (status = 200, body = super::workspaces::WorkspaceResponse),
        (status = 400, body = super::ErrorEnvelope),
        (status = 401, body = super::ErrorEnvelope),
        (status = 403, body = super::ErrorEnvelope),
        (status = 404, body = super::ErrorEnvelope),
        (status = 409, body = super::ErrorEnvelope),
        (status = 422, body = super::ErrorEnvelope),
        (status = 502, body = super::ErrorEnvelope),
        (status = 503, body = super::ErrorEnvelope)
    )
)]
pub(super) async fn update(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(workspace_id): Path<Uuid>,
    Json(request): Json<CorrectWorkspaceHomeCapacityRequest>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.may_manage_system() || actor.has_template_restriction() {
        return Err(ApiError::Forbidden);
    }
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&(workspace_id, &request))?;
    let scope = format!("{}:workspace-home-capacity-correction", actor.user_id);
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
    let workspace = match correct(&state, workspace_id, &request, actor.user_id).await {
        Ok(workspace) => workspace,
        Err(error) => {
            state
                .database
                .abandon_idempotency(&scope, key, &request_hash)
                .await?;
            return Err(error);
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

async fn correct(
    state: &AppState,
    workspace_id: Uuid,
    request: &CorrectWorkspaceHomeCapacityRequest,
    actor: Uuid,
) -> Result<Workspace, ApiError> {
    let now = unix_timestamp()?;
    let lease_owner = format!("home-capacity-api:{}", Uuid::now_v7());
    if !state
        .database
        .try_acquire_workspace_lease(workspace_id, &lease_owner, now, Duration::from_secs(60))
        .await?
    {
        return Err(StorageError::WorkspaceHomePvcUpdateConflict.into());
    }
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let workspace = state.database.get_workspace(workspace_id).await?;
        if workspace.state != WorkspaceState::Stopped
            || workspace.home_volume_binding.is_some()
            || workspace.generation != request.expected_generation
        {
            return Err(StorageError::WorkspaceHomePvcUpdateConflict.into());
        }
        let binding = WorkspaceHomeVolumeBinding {
            namespace: workspace.runtime.namespace().to_owned(),
            claim_name: format!("workspace-data-w-{}-0", workspace.short_id),
            claim_uid: request.claim_uid.clone(),
            capacity_gib: request.capacity_gib,
        };
        if !binding.is_valid() {
            return Err(StorageError::InvalidWorkspaceHomePvc.into());
        }
        let client = state
            .kubernetes_client
            .as_ref()
            .ok_or(ApiError::KubernetesUnavailable)?;
        validation::validate(
            client,
            &workspace,
            &binding,
            state.config.installation_id.as_str(),
        )
        .await?;
        Ok(state
            .database
            .correct_stopped_workspace_home_capacity(
                workspace_id,
                request.capacity_gib,
                request.expected_generation,
                actor,
                unix_timestamp()?,
                &request.claim_uid,
            )
            .await?)
    })
    .await
    .unwrap_or(Err(ApiError::KubernetesUnavailable));
    state
        .database
        .release_workspace_lease(workspace_id, &lease_owner)
        .await?;
    result
}
