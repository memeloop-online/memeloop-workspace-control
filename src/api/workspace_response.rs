use axum::{http::StatusCode, response::Response};

use crate::{
    injections::{
        InjectionScope, InjectionSelection, ResolvedInjectionSummary, filter_injection_refs,
        resolve_injections, select_injections,
    },
    storage::InjectionScopeRef,
    workspaces::{AccessMode, Workspace, WorkspaceState},
};

use super::{
    ApiError, AppState,
    idempotency::json_response,
    workspaces::{
        SshPortStrategy, WorkspaceAppSshConnection, WorkspaceResponse, WorkspaceSshConnection,
    },
};

pub(super) async fn complete_workspace_response(
    state: &AppState,
    scope: &str,
    key: &str,
    request_hash: &str,
    status: StatusCode,
    workspace: Workspace,
    expose_connection: bool,
) -> Result<Response, ApiError> {
    let response_json =
        serde_json::to_string(&workspace_response(state, workspace, expose_connection).await?)
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
    expose_connection: bool,
) -> Result<WorkspaceResponse, ApiError> {
    let namespace = state
        .config
        .installation_id
        .workspace_namespace(&workspace.short_id)
        .map_err(|_| ApiError::BadRequest("workspace namespace is invalid"))?;
    let connectable = expose_connection && workspace.state == WorkspaceState::Ready;
    let internal_endpoint = ssh_endpoint(state, &workspace, &namespace, connectable).await?;
    let (ssh_command, ssh_config) = ssh_commands(
        &workspace,
        internal_endpoint.as_ref(),
        state.config.ssh_public_host.as_deref(),
        connectable,
    );
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
    let jump_host_key = (connectable && workspace.template.access_mode == AccessMode::Public)
        .then(|| state.jump_host_public_key.clone())
        .flatten();
    let ssh_connection = structured_ssh_connection(
        &workspace,
        internal_endpoint.as_ref(),
        ssh_command.as_ref(),
        ssh_config.as_ref(),
        connectable,
    );
    Ok(WorkspaceResponse {
        workspace,
        namespace,
        ssh_connection,
        ssh_host: connectable
            .then(|| internal_endpoint.as_ref().map(|(host, _)| host.clone()))
            .flatten(),
        ssh_port: connectable
            .then(|| internal_endpoint.as_ref().map(|(_, port)| *port))
            .flatten(),
        ssh_command,
        ssh_config,
        web_shell_url,
        injection_sources,
        workspace_host_key,
        jump_host_key,
    })
}

async fn ssh_endpoint(
    state: &AppState,
    workspace: &Workspace,
    namespace: &str,
    connectable: bool,
) -> Result<Option<(String, u16)>, ApiError> {
    let cluster = || (format!("workspace.{namespace}.svc.cluster.local"), 2222);
    if workspace.template.access_mode == AccessMode::Public {
        return Ok(Some(cluster()));
    }
    let Some(host) = state.config.internal_ssh_host.as_ref() else {
        return Ok(Some(cluster()));
    };
    if !connectable {
        return Ok(None);
    }
    let client = state
        .kubernetes_client
        .clone()
        .ok_or(ApiError::KubernetesUnavailable)?;
    Ok(
        crate::kubernetes::workspace_ssh_node_port(client, namespace)
            .await
            .map_err(ApiError::Kubernetes)?
            .map(|port| (host.clone(), port)),
    )
}

fn ssh_commands(
    workspace: &Workspace,
    endpoint: Option<&(String, u16)>,
    public_host: Option<&str>,
    connectable: bool,
) -> (Option<String>, Option<String>) {
    if !connectable {
        return (None, None);
    }
    let Some((host, port)) = endpoint else {
        return (None, None);
    };
    let user = workspace.template.workspace_user.as_str();
    let short = workspace.short_id.as_str();
    match (workspace.template.access_mode, public_host) {
        (AccessMode::Public, Some(jump_host)) => {
            let jump_login = format!("access+{short}@{jump_host}");
            (
                Some(format!("ssh -J {jump_login} -p {port} {user}@{host}")),
                Some(format!(
                    "Host mwc-{short}\n  HostName {host}\n  Port {port}\n  User {user}\n  ProxyJump {jump_login}\n  HostKeyAlias workspace-{short}\n"
                )),
            )
        }
        (AccessMode::Internal, _) => (
            Some(format!("ssh -p {port} {user}@{host}")),
            Some(format!(
                "Host mwc-{short}\n  HostName {host}\n  Port {port}\n  User {user}\n  HostKeyAlias workspace-{short}\n"
            )),
        ),
        (AccessMode::Public, None) => (None, None),
    }
}

fn structured_ssh_connection(
    workspace: &Workspace,
    endpoint: Option<&(String, u16)>,
    command: Option<&String>,
    config: Option<&String>,
    connectable: bool,
) -> Option<WorkspaceSshConnection> {
    let ((hostname, port), command, config) = endpoint
        .zip(command)
        .zip(config)
        .map(|(((hostname, port), command), config)| ((hostname, port), command, config))?;
    if !connectable {
        return None;
    }
    let alias = format!("mwc-{}", workspace.short_id);
    Some(WorkspaceSshConnection {
        display_name: workspace.name.clone(),
        alias: alias.clone(),
        hostname: hostname.clone(),
        port: *port,
        user: workspace.template.workspace_user.clone(),
        command: command.clone(),
        config: config.clone(),
        app: WorkspaceAppSshConnection {
            display_name: workspace.name.clone(),
            hostname: alias,
            ssh_port: None,
            port_strategy: SshPortStrategy::SshConfig,
        },
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
        image: &workspace.template.image,
        access_mode: workspace.template.access_mode,
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
