use k8s_openapi::api::{
    core::v1::{PersistentVolumeClaim, Pod},
    storage::v1::StorageClass,
};
use kube::{
    Api, Client,
    api::{Patch, PatchParams},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use utoipa::ToSchema;

use super::KubernetesCoordinator;
use super::{home_volume_capacity_bytes, ownership};
use crate::{
    config::InstallationId, workspace_runtime::WorkspaceRuntimeNames, workspaces::Workspace,
};

const GIB: u64 = 1 << 30;
const GROWN_TO: &str = "workspace.memeloop.dev/grown-to-gib";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GrowthVolume {
    Home,
    TemporaryStorage,
}

impl GrowthVolume {
    pub(crate) fn capacity(self, workspace: &Workspace) -> u64 {
        match self {
            Self::Home => workspace.template.resources.disk_gib,
            Self::TemporaryStorage => workspace.template.storage_policy.temporary_storage_gib,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct GrowthClaim {
    pub name: String,
    pub uid: String,
}

#[derive(Debug, Error)]
pub(crate) enum GrowthError {
    #[error("the current workspace disk could not be identified")]
    ClaimChanged,
    #[error("online growth is unavailable for this storage class")]
    Unsupported,
    #[error("the current disk size changed")]
    CapacityChanged,
    #[error("Kubernetes rejected the PVC expansion")]
    Rejected,
    #[error(transparent)]
    Kubernetes(#[from] kube::Error),
}

impl KubernetesCoordinator {
    pub(crate) async fn grow_workspace_claim(
        &self,
        workspace: &Workspace,
        target: &GrowthClaim,
        previous_gib: u64,
        requested_gib: u64,
    ) -> Result<bool, GrowthError> {
        grow_claim(
            &self.client,
            workspace.runtime.namespace(),
            target,
            previous_gib,
            requested_gib,
        )
        .await
    }
}

pub(crate) async fn select_claim(
    client: &Client,
    installation: &InstallationId,
    workspace: &Workspace,
    volume: GrowthVolume,
) -> Result<GrowthClaim, GrowthError> {
    let names =
        WorkspaceRuntimeNames::for_workspace(installation, &workspace.runtime, &workspace.short_id)
            .map_err(|_| GrowthError::ClaimChanged)?
            .resources;
    let (volume_name, default_claim, binding) = match volume {
        GrowthVolume::Home => (
            names.data_claim_template.as_str(),
            names.data_pvc_ordinal_zero(),
            workspace.home_volume_binding.as_ref(),
        ),
        GrowthVolume::TemporaryStorage => (
            "workspace-scratch",
            names.scratch_pvc(),
            workspace.scratch_volume_binding.as_ref(),
        ),
    };
    let name = binding.map_or(default_claim, |binding| binding.claim_name.clone());
    let ns = workspace.runtime.namespace();
    let pods = Api::<Pod>::namespaced(client.clone(), ns);
    let claims = Api::<PersistentVolumeClaim>::namespaced(client.clone(), ns);
    let pod_name = names.pod_ordinal_zero();
    let (pod, claim) = tokio::try_join!(pods.get_opt(&pod_name), claims.get_opt(&name))?;
    let pod = pod.ok_or(GrowthError::ClaimChanged)?;
    let claim = claim.ok_or(GrowthError::ClaimChanged)?;
    let owns = |metadata| {
        ownership::verify(metadata, installation.as_str(), &workspace.id.to_string()).is_ok()
    };
    if pod.metadata.deletion_timestamp.is_some()
        || !owns(&pod.metadata)
        || !pod_mounts_claim(&pod, volume_name, &name)
        || binding.is_none() && !owns(&claim.metadata)
    {
        return Err(GrowthError::ClaimChanged);
    }
    let uid = claim
        .metadata
        .uid
        .clone()
        .ok_or(GrowthError::ClaimChanged)?;
    if binding.is_some_and(|binding| binding.claim_uid != uid) {
        return Err(GrowthError::ClaimChanged);
    }
    let (requested, applied) = claim_sizes(&claim, &uid)?;
    let current = volume.capacity(workspace) * GIB;
    if requested != current || applied < current {
        return Err(GrowthError::CapacityChanged);
    }
    let class_name = claim
        .spec
        .as_ref()
        .and_then(|spec| spec.storage_class_name.as_deref())
        .ok_or(GrowthError::Unsupported)?;
    let class = Api::<StorageClass>::all(client.clone())
        .get(class_name)
        .await?;
    if class.allow_volume_expansion != Some(true) || class.provisioner != "topolvm.io" {
        return Err(GrowthError::Unsupported);
    }
    Ok(GrowthClaim { name, uid })
}

fn pod_mounts_claim(pod: &Pod, volume_name: &str, claim_name: &str) -> bool {
    pod.spec.as_ref().is_some_and(|spec| {
        spec.volumes.iter().flatten().any(|volume| {
            volume.name == volume_name
                && volume
                    .persistent_volume_claim
                    .as_ref()
                    .is_some_and(|claim| {
                        claim.claim_name == claim_name && claim.read_only != Some(true)
                    })
        }) && spec.containers.iter().any(|container| {
            container.name == "workspace"
                && container
                    .volume_mounts
                    .iter()
                    .flatten()
                    .any(|mount| mount.name == volume_name && mount.read_only != Some(true))
        })
    })
}

/// Requests growth of the selected PVC and reports when the filesystem has grown.
/// Retrying after a controller restart does not patch a second time.
pub(crate) async fn grow_claim(
    client: &Client,
    namespace: &str,
    target: &GrowthClaim,
    previous_gib: u64,
    requested_gib: u64,
) -> Result<bool, GrowthError> {
    let claims = Api::<PersistentVolumeClaim>::namespaced(client.clone(), namespace);
    let current = claims
        .get_opt(&target.name)
        .await?
        .ok_or(GrowthError::ClaimChanged)?;
    let (requested, applied) = claim_sizes(&current, &target.uid)?;
    let required = requested_gib * GIB;
    if requested == required {
        let pending = current
            .status
            .as_ref()
            .and_then(|status| status.conditions.as_ref())
            .into_iter()
            .flatten()
            .any(|condition| {
                condition.status == "True"
                    && matches!(
                        condition.type_.as_str(),
                        "FileSystemResizePending" | "Resizing"
                    )
            });
        return Ok(applied >= required && !pending);
    }
    if requested != previous_gib * GIB {
        return Err(GrowthError::CapacityChanged);
    }
    let resource_version = current
        .metadata
        .resource_version
        .as_ref()
        .ok_or(GrowthError::ClaimChanged)?;
    let result = claims
        .patch(
            &target.name,
            &PatchParams::default(),
            &Patch::Merge(&json!({
                "metadata": { "uid": target.uid, "resourceVersion": resource_version,
                    "annotations": { (GROWN_TO): requested_gib.to_string() } },
                "spec": { "resources": { "requests": { "storage": format!("{requested_gib}Gi") } } }
            })),
        )
        .await;
    match result {
        Ok(_) => Ok(false),
        Err(kube::Error::Api(error)) if matches!(error.code, 403 | 422) => {
            Err(GrowthError::Rejected)
        }
        Err(error) => Err(error.into()),
    }
}

pub(super) fn claim_sizes(
    claim: &PersistentVolumeClaim,
    uid: &str,
) -> Result<(u64, u64), GrowthError> {
    if claim.metadata.uid.as_deref() != Some(uid) || claim.metadata.deletion_timestamp.is_some() {
        return Err(GrowthError::ClaimChanged);
    }
    let requested = claim.spec.as_ref().and_then(|spec| {
        if spec
            .volume_mode
            .as_deref()
            .is_some_and(|mode| mode != "Filesystem")
            || spec.volume_name.as_deref().is_none_or(str::is_empty)
        {
            return None;
        }
        spec.resources.as_ref()?.requests.as_ref()?.get("storage")
    });
    let applied = claim.status.as_ref().and_then(|status| {
        if status.phase.as_deref() != Some("Bound") {
            return None;
        }
        status.capacity.as_ref()?.get("storage")
    });
    requested
        .and_then(|quantity| home_volume_capacity_bytes(&quantity.0))
        .zip(applied.and_then(|quantity| home_volume_capacity_bytes(&quantity.0)))
        .ok_or(GrowthError::ClaimChanged)
}

pub(super) fn matches_applied_growth(claim: &PersistentVolumeClaim, gib: u64) -> bool {
    claim
        .metadata
        .annotations
        .as_ref()
        .and_then(|annotations| annotations.get(GROWN_TO))
        .and_then(|value| value.parse::<u64>().ok())
        == Some(gib)
        && claim
            .metadata
            .uid
            .as_deref()
            .and_then(|uid| claim_sizes(claim, uid).ok())
            .is_some_and(|(requested, applied)| requested == gib * GIB && applied >= requested)
}

#[cfg(test)]
mod tests {
    use std::{
        convert::Infallible,
        sync::{Arc, Mutex},
    };

    use axum::http::{Method, Request, Response};
    use http_body_util::BodyExt;
    use serde_json::Value;
    use tower::service_fn;

    use super::*;

    #[tokio::test]
    async fn growth_patches_only_the_selected_pvc_and_waits_for_its_filesystem() {
        let pvc = Arc::new(Mutex::new(json!({
            "apiVersion":"v1","kind":"PersistentVolumeClaim",
            "metadata":{"name":"disk","uid":"original-pvc","resourceVersion":"12"},
            "spec":{"volumeName":"pv-original","resources":{"requests":{"storage":"4Gi"}}},
            "status":{"phase":"Bound","capacity":{"storage":"4Gi"}}
        })));
        let patches = Arc::new(Mutex::new(0));
        let client = Client::new(
            service_fn({
                let (pvc, patches) = (pvc.clone(), patches.clone());
                move |request: Request<kube::client::Body>| {
                    let (pvc, patches) = (pvc.clone(), patches.clone());
                    async move {
                        assert_eq!(
                            request.uri().path(),
                            "/api/v1/namespaces/test/persistentvolumeclaims/disk"
                        );
                        if request.method() == Method::PATCH {
                            let patch: Value = serde_json::from_slice(
                                &request.into_body().collect().await.unwrap().to_bytes(),
                            )
                            .unwrap();
                            assert_eq!(patch["metadata"]["uid"], "original-pvc");
                            assert_eq!(patch["metadata"]["resourceVersion"], "12");
                            assert_eq!(patch["spec"]["resources"]["requests"]["storage"], "8Gi");
                            pvc.lock().unwrap()["spec"]["resources"]["requests"]["storage"] =
                                json!("8Gi");
                            *patches.lock().unwrap() += 1;
                        } else {
                            assert_eq!(request.method(), Method::GET);
                        }
                        Ok::<_, Infallible>(Response::new(axum::body::Body::from(
                            pvc.lock().unwrap().to_string(),
                        )))
                    }
                }
            }),
            "test",
        );
        let identity = GrowthClaim {
            name: "disk".to_owned(),
            uid: "original-pvc".to_owned(),
        };
        assert!(!grow_claim(&client, "test", &identity, 4, 8).await.unwrap());
        assert!(!grow_claim(&client, "test", &identity, 4, 8).await.unwrap());
        pvc.lock().unwrap()["status"] = json!({
            "phase":"Bound","capacity":{"storage":"8Gi"},
            "conditions":[{"type":"FileSystemResizePending","status":"True"}]
        });
        assert!(!grow_claim(&client, "test", &identity, 4, 8).await.unwrap());
        pvc.lock().unwrap()["status"]["conditions"] = json!([]);
        assert!(grow_claim(&client, "test", &identity, 4, 8).await.unwrap());
        pvc.lock().unwrap()["metadata"]["uid"] = json!("a-replacement-pvc");
        assert!(matches!(
            grow_claim(&client, "test", &identity, 4, 8).await,
            Err(GrowthError::ClaimChanged)
        ));
        assert_eq!(*patches.lock().unwrap(), 1);
    }
}
