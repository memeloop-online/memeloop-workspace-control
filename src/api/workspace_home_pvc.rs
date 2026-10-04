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
pub(super) struct BindWorkspaceHomePvcRequest {
    pub namespace: String,
    pub claim_name: String,
    pub claim_uid: String,
    pub capacity_gib: u64,
    pub expected_generation: u64,
}

impl BindWorkspaceHomePvcRequest {
    fn binding(&self) -> WorkspaceHomeVolumeBinding {
        WorkspaceHomeVolumeBinding {
            namespace: self.namespace.clone(),
            claim_name: self.claim_name.clone(),
            claim_uid: self.claim_uid.clone(),
            capacity_gib: self.capacity_gib,
        }
    }
}

#[utoipa::path(
    put,
    path = "/api/v1/workspaces/{workspace_id}/home-pvc",
    request_body = BindWorkspaceHomePvcRequest,
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
    Json(request): Json<BindWorkspaceHomePvcRequest>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.may_manage_system() || actor.has_template_restriction() {
        return Err(ApiError::Forbidden);
    }
    let binding = request.binding();
    if !binding.is_valid() {
        return Err(StorageError::InvalidWorkspaceHomePvc.into());
    }
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&(workspace_id, &request))?;
    let scope = format!("{}:workspace-home-pvc-binding", actor.user_id);
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
    let workspace = match bind(
        &state,
        workspace_id,
        &binding,
        request.expected_generation,
        actor.user_id,
    )
    .await
    {
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

async fn bind(
    state: &AppState,
    workspace_id: Uuid,
    binding: &WorkspaceHomeVolumeBinding,
    generation: u64,
    actor: Uuid,
) -> Result<Workspace, ApiError> {
    let now = unix_timestamp()?;
    let lease_owner = format!("home-pvc-api:{}", Uuid::now_v7());
    if !state
        .database
        .try_acquire_workspace_lease(workspace_id, &lease_owner, now, Duration::from_secs(60))
        .await?
    {
        return Err(StorageError::WorkspaceHomePvcUpdateConflict.into());
    }
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let workspace = state.database.get_workspace(workspace_id).await?;
        if !matches!(
            workspace.state,
            WorkspaceState::Stopped | WorkspaceState::Ready | WorkspaceState::Failed
        ) || workspace.generation != generation
        {
            return Err(StorageError::WorkspaceHomePvcUpdateConflict.into());
        }
        let client = state
            .kubernetes_client
            .as_ref()
            .ok_or(ApiError::KubernetesUnavailable)?;
        validation::validate(
            client,
            &workspace,
            binding,
            state.config.installation_id.as_str(),
        )
        .await?;
        Ok(state
            .database
            .bind_workspace_home_volume(
                workspace_id,
                binding,
                generation,
                actor,
                unix_timestamp()?,
                workspace.state != WorkspaceState::Stopped,
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
