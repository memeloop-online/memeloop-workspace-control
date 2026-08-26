use axum::{http::StatusCode, response::Response};

use crate::{
    injections::{
        InjectionScope, InjectionSelection, ResolvedInjectionSummary, filter_injection_refs,
        resolve_injections, select_injections,
    },
    storage::InjectionScopeRef,
    workspaces::{AccessMode, Workspace, WorkspaceState},
};

use super::{ApiError, AppState, idempotency::json_response, workspaces::WorkspaceResponse};

pub(super) async fn complete_workspace_response(
    state: &AppState,
    scope: &str,
    key: &str,
    request_hash: &str,
    status: StatusCode,
    workspace: Workspace,
) -> Result<Response, ApiError> {
    let response_json = serde_json::to_string(&workspace_response(state, workspace).await?)
        .map_err(|_| ApiError::BadRequest("response serialization failed"))?;
    state
        .database
        .finish_idempotency(scope, key, request_hash, status.as_u16(), &response_json)
        .await?;
    json_response(status, response_json)
}

pub(super) async fn workspace_response(
    state: &AppState,
    workspace: Workspace,
) -> Result<WorkspaceResponse, ApiError> {
    let namespace = state
        .config
        .installation_id
        .workspace_namespace(&workspace.short_id)
        .map_err(|_| ApiError::BadRequest("workspace namespace is invalid"))?;
    let connectable = workspace.state == WorkspaceState::Ready;
    let internal_host = format!("workspace.{namespace}.svc.cluster.local");
    let (ssh_command, ssh_config) = if connectable {
        match (
            workspace.access_mode,
            state.config.ssh_public_host.as_deref(),
        ) {
            (AccessMode::Public, Some(jump_host)) => {
                let jump_login = format!("access+{}@{jump_host}", workspace.short_id);
                (
                    Some(format!(
                        "ssh -J {jump_login} -p 2222 workspace@{internal_host}"
                    )),
                    Some(format!(
                        "Host mwc-{short}\n  HostName {internal_host}\n  Port 2222\n  User workspace\n  ProxyJump {jump_login}\n  HostKeyAlias workspace-{short}\n",
                        short = workspace.short_id
                    )),
                )
            }
            (AccessMode::Internal, _) => (
                Some(format!("ssh -p 2222 workspace@{internal_host}")),
                Some(format!(
                    "Host mwc-{short}\n  HostName {internal_host}\n  Port 2222\n  User workspace\n  HostKeyAlias workspace-{short}\n",
                    short = workspace.short_id
                )),
            ),
            (AccessMode::Public, None) => (None, None),
        }
    } else {
        (None, None)
    };
    let web_shell_url = connectable
        .then(|| {
            state
                .config
                .web_shell_public_origin
                .as_ref()
                .map(|origin| format!("{origin}/shell/{}/", workspace.short_id))
        })
        .flatten();
    let injection_sources = injection_sources(state, &workspace).await?;
    let workspace_host_key = if connectable {
        state
            .database
            .workspace_ssh_public_identity(workspace.id)
            .await?
    } else {
        None
    };
    let jump_host_key = (connectable && workspace.access_mode == AccessMode::Public)
        .then(|| state.jump_host_public_key.clone())
        .flatten();
    Ok(WorkspaceResponse {
        workspace,
        namespace,
        ssh_command,
        ssh_config,
        web_shell_url,
        injection_sources,
        workspace_host_key,
        jump_host_key,
    })
}

async fn injection_sources(
    state: &AppState,
    workspace: &Workspace,
) -> Result<Vec<ResolvedInjectionSummary>, ApiError> {
    let Some(cipher) = state.cipher.as_ref() else {
        return Ok(Vec::new());
    };
    let load = |scope, scope_id| {
        state
            .database
            .load_injections(cipher, InjectionScopeRef { scope, scope_id })
    };
    let organization = load(InjectionScope::Organization, workspace.organization_id).await?;
    let user = load(InjectionScope::User, workspace.owner_id).await?;
    let workspace_items = load(InjectionScope::Workspace, workspace.id).await?;
    let selection = InjectionSelection {
        workspace_id: Some(workspace.id),
        organization_id: workspace.organization_id,
        owner_id: workspace.owner_id,
        template_id: workspace.template_id,
        image: &workspace.image,
        access_mode: workspace.access_mode,
    };
    let refs = state
        .database
        .workspace_injection_refs(workspace.id)
        .await?;
    Ok(resolve_injections(
        &filter_injection_refs(
            &select_injections(&organization, selection),
            refs.organization.as_deref(),
            true,
        ),
        &filter_injection_refs(
            &select_injections(&user, selection),
            refs.user.as_deref(),
            false,
        ),
        &select_injections(&workspace_items, selection),
    )?
    .into_iter()
    .map(|item| item.summary())
    .collect())
}
