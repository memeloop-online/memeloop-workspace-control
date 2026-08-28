use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    auth::Permission,
    injections::{InjectionItem, ResolvedInjectionSummary},
    storage::{CreateWorkspace, IdempotencyDecision},
    workspaces::{Workspace, WorkspaceAction},
};

use super::{
    ApiError, AppState,
    auth::principal,
    idempotency::{
        IDEMPOTENCY_TTL_SECONDS, hash, idempotency_key, replay_response, unix_timestamp,
    },
    workspace_creation::{validate_inline_injections, wait_until_ready},
    workspace_response::{complete_workspace_response, workspace_response},
};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(super) struct CreateWorkspaceRequest {
    #[serde(flatten)]
    pub workspace: CreateWorkspace,
    #[serde(default)]
    pub inline_workspace_injections: Vec<InjectionItem>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub(super) struct CreateWorkspaceQuery {
    pub wait_until: Option<String>,
    pub timeout: Option<u64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct WorkspaceResponse {
    pub workspace: Workspace,
    pub namespace: String,
    pub ssh_host: Option<String>,
    pub ssh_port: Option<u16>,
    pub ssh_command: Option<String>,
    pub ssh_config: Option<String>,
    pub web_shell_url: Option<String>,
    pub injection_sources: Vec<ResolvedInjectionSummary>,
    pub workspace_host_key: Option<crate::storage::WorkspaceSshPublicIdentity>,
    pub jump_host_key: Option<crate::storage::WorkspaceSshPublicIdentity>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub(super) struct WorkspaceListQuery {
    pub organization_id: Uuid,
}

#[utoipa::path(
    post,
    path = "/api/v1/workspaces",
    request_body = CreateWorkspaceRequest,
    params(CreateWorkspaceQuery, ("Idempotency-Key" = String, Header, description = "Unique key for this request")),
    responses(
        (status = 201, description = "Workspace accepted for provisioning", body = WorkspaceResponse),
        (status = 401, body = super::ErrorEnvelope),
        (status = 403, body = super::ErrorEnvelope),
        (status = 409, body = super::ErrorEnvelope),
        (status = 422, body = super::ErrorEnvelope)
    )
)]
pub(super) async fn create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<CreateWorkspaceQuery>,
    Json(request): Json<CreateWorkspaceRequest>,
) -> Result<Response, ApiError> {
    if query
        .wait_until
        .as_deref()
        .is_some_and(|value| value != "ready")
    {
        return Err(ApiError::BadRequest("wait_until must be ready"));
    }
    let command = request.workspace.clone();
    let actor = principal(&state, &headers).await?;
    if !actor.allows(Permission::CreateWorkspace, command.organization_id) {
        return Err(ApiError::Forbidden);
    }
    if command.owner_id != actor.user_id
        && !actor.allows(Permission::ManageMembers, command.organization_id)
    {
        return Err(ApiError::Forbidden);
    }
    let template = state
        .database
        .get_workspace_template(command.template_id)
        .await?;
    if !template.enabled
        || template
            .organization_id
            .is_some_and(|id| id != command.organization_id)
    {
        return Err(crate::storage::StorageError::TemplateNotFound.into());
    }
    if template.template.cluster_access && !actor.system_admin {
        return Err(ApiError::Forbidden);
    }
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&serde_json::json!({
        "request": &request,
        "wait_until": &query.wait_until,
        "timeout": query.timeout,
    }))?;
    let scope = format!("{}:create-workspace", actor.user_id);
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

    if let Err(error) =
        validate_inline_injections(&state, &command, &request.inline_workspace_injections).await
    {
        state
            .database
            .abandon_idempotency(&scope, key, &request_hash)
            .await?;
        return Err(error);
    }

    let creation = if request.inline_workspace_injections.is_empty() {
        state
            .database
            .create_workspace(command, actor.user_id, now)
            .await
    } else {
        let cipher = state
            .cipher
            .as_ref()
            .ok_or(ApiError::EncryptionUnavailable)?;
        state
            .database
            .create_workspace_with_inline_injections(
                command,
                cipher,
                &request.inline_workspace_injections,
                actor.user_id,
                now,
            )
            .await
    };
    let mut workspace = match creation {
        Ok(workspace) => workspace,
        Err(error) => {
            state
                .database
                .abandon_idempotency(&scope, key, &request_hash)
                .await?;
            return Err(error.into());
        }
    };
    if query.wait_until.as_deref() == Some("ready") {
        workspace =
            wait_until_ready(&state, workspace, query.timeout.unwrap_or(30).clamp(1, 120)).await?;
    }
    complete_workspace_response(
        &state,
        &scope,
        key,
        &request_hash,
        StatusCode::CREATED,
        workspace,
    )
    .await
}

#[utoipa::path(
    get,
    path = "/api/v1/workspaces",
    params(WorkspaceListQuery),
    responses(
        (status = 200, description = "Visible workspaces", body = [WorkspaceResponse]),
        (status = 401, body = super::ErrorEnvelope),
        (status = 403, body = super::ErrorEnvelope)
    )
)]
pub(super) async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<WorkspaceListQuery>,
) -> Result<Json<Vec<WorkspaceResponse>>, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.allows(Permission::ReadWorkspace, query.organization_id) {
        return Err(ApiError::Forbidden);
    }
    let workspaces = state
        .database
        .list_workspaces(query.organization_id)
        .await?;
    let mut responses = Vec::with_capacity(workspaces.len());
    for workspace in workspaces {
        responses.push(workspace_response(&state, workspace).await?);
    }
    Ok(Json(responses))
}

#[utoipa::path(
    get,
    path = "/api/v1/workspaces/{workspace_id}",
    params(("workspace_id" = Uuid, Path)),
    responses(
        (status = 200, body = WorkspaceResponse),
        (status = 401, body = super::ErrorEnvelope),
        (status = 403, body = super::ErrorEnvelope),
        (status = 404, body = super::ErrorEnvelope)
    )
)]
pub(super) async fn get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<WorkspaceResponse>, ApiError> {
    let actor = principal(&state, &headers).await?;
    let workspace = state.database.get_workspace(workspace_id).await?;
    if !actor.allows(Permission::ReadWorkspace, workspace.organization_id) {
        return Err(ApiError::Forbidden);
    }
    Ok(Json(workspace_response(&state, workspace).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/workspaces/{workspace_id}/actions/{action}",
    params(
        ("workspace_id" = Uuid, Path),
        ("action" = String, Path, description = "start, stop, restart, or delete"),
        ("Idempotency-Key" = String, Header)
    ),
    responses(
        (status = 202, body = WorkspaceResponse),
        (status = 401, body = super::ErrorEnvelope),
        (status = 403, body = super::ErrorEnvelope),
        (status = 409, body = super::ErrorEnvelope)
    )
)]
pub(super) async fn action(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((workspace_id, action_name)): Path<(Uuid, String)>,
) -> Result<Response, ApiError> {
    let action = WorkspaceAction::from_api(&action_name).ok_or(ApiError::BadRequest(
        "action must be start, stop, restart, or delete",
    ))?;
    let actor = principal(&state, &headers).await?;
    let existing = state.database.get_workspace(workspace_id).await?;
    let permission = if action == WorkspaceAction::Delete {
        Permission::DeleteWorkspace
    } else {
        Permission::ChangeWorkspaceState
    };
    if !actor.allows(permission, existing.organization_id) {
        return Err(ApiError::Forbidden);
    }
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&serde_json::json!({
        "workspace_id": workspace_id,
        "action": action.as_str()
    }))?;
    let scope = format!("{}:workspace-action", actor.user_id);
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
        .transition_workspace(workspace_id, action, actor.user_id, now)
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
        StatusCode::ACCEPTED,
        workspace,
    )
    .await
}
