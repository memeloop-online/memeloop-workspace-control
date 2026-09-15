use std::{sync::Arc, time::Duration};

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use k8s_openapi::api::core::v1::Pod;
use kube::{Api, api::AttachParams};
use serde::Serialize;
use ssh_key::PublicKey;
use tokio::io::AsyncReadExt;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{auth::Permission, workspaces::WorkspaceState};

use super::{ApiError, AppState, auth::principal};

const MAX_PUBLIC_KEY_BYTES: u64 = 8 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct WorkspaceClientPublicKey {
    pub public_key: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/workspaces/{workspace_id}/ssh-client-public-key",
    params(("workspace_id" = Uuid, Path)),
    responses(
        (status = 200, body = WorkspaceClientPublicKey),
        (status = 401, body = super::ErrorEnvelope),
        (status = 403, body = super::ErrorEnvelope),
        (status = 404, body = super::ErrorEnvelope),
        (status = 409, body = super::ErrorEnvelope),
        (status = 503, body = super::ErrorEnvelope)
    )
)]
pub(super) async fn get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<WorkspaceClientPublicKey>, ApiError> {
    let actor = principal(&state, &headers).await?;
    let workspace = state.database.get_workspace(workspace_id).await?;
    if !actor.allows(Permission::ConnectWorkspace, workspace.organization_id)
        || !actor.may_access_workspace_template(workspace.template_id)
    {
        return Err(ApiError::Forbidden);
    }
    if workspace.state != WorkspaceState::Ready {
        return Err(ApiError::WorkspaceNotConnectable);
    }
    let client = state
        .kubernetes_client
        .clone()
        .ok_or(ApiError::KubernetesUnavailable)?;
    let names = crate::workspace_runtime::WorkspaceRuntimeNames::for_workspace(
        &state.config.installation_id,
        &workspace.runtime,
        &workspace.short_id,
    )
    .map_err(|_| ApiError::WorkspaceClientKeyUnavailable)?;
    let pod_name = format!("{}-0", names.resources.stateful_set);
    let key_path = format!("{}/.ssh/id_ed25519.pub", workspace.template.workspace_home);
    let raw = read_public_key(client, names.namespace.as_str(), &pod_name, &key_path).await?;
    let comment = format!("{}@{pod_name}", workspace.template.workspace_user);
    let public_key = normalize_public_key(&raw, &comment)?;
    Ok(Json(WorkspaceClientPublicKey { public_key }))
}

async fn read_public_key(
    client: kube::Client,
    namespace: &str,
    pod_name: &str,
    path: &str,
) -> Result<String, ApiError> {
    let pods = Api::<Pod>::namespaced(client, namespace);
    let params = AttachParams::default()
        .container("workspace")
        .stderr(false)
        .max_stdout_buf_size(MAX_PUBLIC_KEY_BYTES as usize + 1);
    let mut process = pods
        .exec(pod_name, ["cat", path], &params)
        .await
        .map_err(ApiError::Kubernetes)?;
    let stdout = process
        .stdout()
        .ok_or(ApiError::WorkspaceClientKeyUnavailable)?;
    let mut bytes = Vec::new();
    tokio::time::timeout(
        READ_TIMEOUT,
        stdout
            .take(MAX_PUBLIC_KEY_BYTES + 1)
            .read_to_end(&mut bytes),
    )
    .await
    .map_err(|_| ApiError::WorkspaceClientKeyUnavailable)?
    .map_err(|_| ApiError::WorkspaceClientKeyUnavailable)?;
    process.join().await.map_err(|error| {
        tracing::warn!(error = %error, pod = pod_name, "workspace public key read failed");
        ApiError::WorkspaceClientKeyUnavailable
    })?;
    if bytes.is_empty() || bytes.len() > MAX_PUBLIC_KEY_BYTES as usize {
        return Err(ApiError::WorkspaceClientKeyUnavailable);
    }
    String::from_utf8(bytes).map_err(|_| ApiError::WorkspaceClientKeyUnavailable)
}

fn normalize_public_key(raw: &str, comment: &str) -> Result<String, ApiError> {
    let mut key =
        PublicKey::from_openssh(raw.trim()).map_err(|_| ApiError::WorkspaceClientKeyUnavailable)?;
    key.set_comment(comment);
    key.to_openssh()
        .map_err(|_| ApiError::WorkspaceClientKeyUnavailable)
}

#[cfg(test)]
mod tests {
    use super::normalize_public_key;

    #[test]
    fn public_key_gets_a_stable_workspace_hostname_comment() {
        let raw =
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMBbdKulBHZ15UIMQzYRvlgFUwkLPEuyYeGl1cdqUoHE old";
        let normalized = normalize_public_key(raw, "user@w-bd2dc9ca6aa2b1b5-0").unwrap();
        assert_eq!(
            normalized,
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMBbdKulBHZ15UIMQzYRvlgFUwkLPEuyYeGl1cdqUoHE user@w-bd2dc9ca6aa2b1b5-0"
        );
    }
}
