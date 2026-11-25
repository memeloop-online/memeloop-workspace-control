use std::fmt::Debug;

use k8s_openapi::api::{
    apps::v1::StatefulSet,
    core::v1::{Pod, Secret, Service},
};
use kube::{Api, Client};
use serde::de::DeserializeOwned;
use thiserror::Error;
use uuid::Uuid;

use super::{
    BuildError, InjectionMaterialization, OwnershipError, ResolvedPlacement, ResourceBuilder,
};
use crate::{
    config::InstallationId,
    workspace_runtime::{WorkspaceRuntimeIdentityError, WorkspaceRuntimeNames},
    workspaces::Workspace,
};

const FIELD_MANAGER: &str = "memeloop-workspace-control";

#[path = "client_apply.rs"]
mod apply;
#[path = "client_delete.rs"]
mod delete;
#[path = "client_home_volume.rs"]
mod home_volume;
#[path = "client_memory_resize.rs"]
mod memory_resize;
#[path = "client_scratch_recovery.rs"]
mod scratch_recovery;
#[path = "client_storage_shrink.rs"]
mod storage_shrink;
pub use home_volume::home_volume_capacity_bytes;
pub(crate) use memory_resize::MemoryResize;
pub(super) use memory_resize::retained_memory_revision;
pub(crate) use storage_shrink::storage_shrink_image;
pub(crate) use storage_shrink::{ShrinkPodStage, ShrinkUsage};

#[derive(Clone)]
pub struct KubernetesCoordinator {
    pub(super) client: Client,
    pub(super) builder: ResourceBuilder,
}

impl KubernetesCoordinator {
    pub fn new(client: Client, builder: ResourceBuilder) -> Self {
        Self { client, builder }
    }

    pub async fn reconcile(&self, workspace: &Workspace) -> Result<(), ReconcileError> {
        let desired = self.builder.build(workspace)?;
        self.apply_desired(workspace, &desired).await
    }

    pub async fn reconcile_with_injections(
        &self,
        workspace: &Workspace,
        placement: &ResolvedPlacement,
        injections: InjectionMaterialization,
        ssh_identity: Secret,
    ) -> Result<(), ReconcileError> {
        let mut desired = self.builder.build_with_placement(workspace, placement)?;
        let workspace_container = desired
            .stateful_set
            .spec
            .as_mut()
            .and_then(|spec| spec.template.spec.as_mut())
            .and_then(|spec| {
                spec.containers
                    .iter_mut()
                    .find(|container| container.name == "workspace")
            })
            .ok_or(ReconcileError::MissingWorkspaceContainer)?;
        super::workspace_pod::apply_injected_environment_overrides(
            &workspace.template,
            workspace_container,
            &injections.environment_targets,
        );
        desired.injections = injections;
        desired.ssh_identity = ssh_identity;
        self.apply_desired(workspace, &desired).await
    }

    pub async fn has_observed_replicas(
        &self,
        workspace: &Workspace,
        expected: i32,
    ) -> Result<bool, ReconcileError> {
        let phase = if expected == 0 {
            "observe_stopped_replicas"
        } else {
            "observe_ready_replicas"
        };
        apply::observe_runtime_stage(
            workspace,
            phase,
            Box::pin(self.has_observed_replicas_inner(workspace, expected, false)),
        )
        .await
    }

    /// Health of the workload currently running, independent of an unapplied edit.
    pub(crate) async fn has_running_workspace_connection(
        &self,
        workspace: &Workspace,
    ) -> Result<bool, ReconcileError> {
        apply::observe_runtime_stage(
            workspace,
            "observe_running_workspace_connection",
            Box::pin(self.has_observed_replicas_inner(workspace, 1, true)),
        )
        .await
    }

    async fn has_observed_replicas_inner(
        &self,
        workspace: &Workspace,
        expected: i32,
        observe_applied_generation: bool,
    ) -> Result<bool, ReconcileError> {
        let namespace_name = workspace.runtime.namespace();
        let names = self.builder.runtime_names(workspace)?;
        let stateful_sets = Api::<StatefulSet>::namespaced(self.client.clone(), namespace_name);
        let pods = Api::<Pod>::namespaced(self.client.clone(), namespace_name);
        let (stateful_set, pod) = tokio::join!(
            apply::observe_runtime_stage(
                workspace,
                "observe_statefulset_read",
                Box::pin(stateful_sets.get_opt(&names.resources.stateful_set)),
            ),
            async {
                if expected != 1 {
                    return Ok(None);
                }
                apply::observe_runtime_stage(
                    workspace,
                    "observe_ready_pod_read",
                    Box::pin(pods.get_opt(&names.resources.pod_ordinal_zero())),
                )
                .await
            }
        );
        let Some(stateful_set) = stateful_set? else {
            return Ok(false);
        };
        self.builder
            .verify_delete_ownership(&stateful_set.metadata, workspace.id)?;
        let retained_revision = observe_applied_generation
            .then(|| retained_memory_revision(&stateful_set))
            .flatten();
        let Some(applied_generation) =
            observed_workspace_generation(&stateful_set, expected, retained_revision.is_some())
        else {
            return Ok(false);
        };
        if applied_generation > workspace.generation
            || (!observe_applied_generation && applied_generation != workspace.generation)
        {
            return Ok(false);
        }
        if expected == 0 {
            return Ok(stateful_set
                .status
                .as_ref()
                .is_some_and(|status| status.replicas == 0));
        }
        let Some(pod) = pod? else {
            return Ok(false);
        };
        self.builder
            .verify_delete_ownership(&pod.metadata, workspace.id)?;
        Ok(expected == 1
            && current_workspace_pod(
                &stateful_set,
                &pod,
                applied_generation,
                &names.resources.stateful_set,
                retained_revision,
            )
            && workspace_connection_ready(&pod))
    }
}

pub(super) fn workspace_connection_ready(pod: &Pod) -> bool {
    let Some(status) = &pod.status else {
        return false;
    };
    if pod.metadata.deletion_timestamp.is_some() || status.phase.as_deref() != Some("Running") {
        return false;
    }
    let conditions = status.conditions.as_deref().unwrap_or_default();
    let Some(ready) = conditions
        .iter()
        .find(|condition| condition.type_ == "Ready")
    else {
        return false;
    };
    if ready.status == "True" {
        return true;
    }
    // The SSH and terminal Services publish endpoints independently of Pod Ready.
    // BuildKit's own probes and runtime status still report its health.
    let (Some(spec), Some(containers)) = (&pod.spec, &status.container_statuses) else {
        return false;
    };
    ready.status == "False"
        && ready.reason.as_deref() == Some("ContainersNotReady")
        && spec
            .readiness_gates
            .as_deref()
            .unwrap_or_default()
            .is_empty()
        && ["Initialized", "PodReadyToStartContainers"]
            .into_iter()
            .all(|required| {
                conditions
                    .iter()
                    .any(|condition| condition.type_ == required && condition.status == "True")
            })
        && ["workspace", "ttyd", "buildkitd"].into_iter().all(|name| {
            spec.containers
                .iter()
                .any(|container| container.name == name)
        })
        && spec.containers.iter().all(|container| {
            containers.iter().any(|status| {
                status.name == container.name && status.ready == (container.name != "buildkitd")
            })
        })
}

fn observed_workspace_generation(
    set: &StatefulSet,
    expected: i32,
    retain_revision: bool,
) -> Option<u64> {
    let (spec, status) = (set.spec.as_ref()?, set.status.as_ref()?);
    if spec.replicas != Some(expected)
        || set.metadata.deletion_timestamp.is_some()
        || (!retain_revision && status.observed_generation? < set.metadata.generation?)
    {
        return None;
    }
    spec.template
        .metadata
        .as_ref()?
        .annotations
        .as_ref()?
        .get("workspace.memeloop.dev/generation")?
        .parse()
        .ok()
}

fn current_workspace_pod(
    set: &StatefulSet,
    pod: &Pod,
    generation: u64,
    set_name: &str,
    retained_revision: Option<&str>,
) -> bool {
    let (Some(uid), Some(revision)) = (
        set.metadata.uid.as_deref(),
        set.status
            .as_ref()
            .and_then(|status| status.update_revision.as_deref()),
    ) else {
        return false;
    };
    pod.metadata.uid.is_some()
        && pod.metadata.deletion_timestamp.is_none()
        && !restart_generation_is_stale(pod, generation)
        && pod
            .metadata
            .owner_references
            .as_ref()
            .is_some_and(|owners| {
                owners.iter().any(|owner| {
                    owner.controller == Some(true)
                        && owner.api_version == "apps/v1"
                        && owner.kind == "StatefulSet"
                        && owner.name == set_name
                        && owner.uid == uid
                })
            })
        && pod.metadata.labels.as_ref().is_some_and(|labels| {
            labels
                .get("controller-revision-hash")
                .is_some_and(|pod_revision| {
                    pod_revision == revision || Some(pod_revision.as_str()) == retained_revision
                })
        })
}

/// Returns the apiserver-assigned SSH NodePort for an internal workspace.
pub async fn workspace_ssh_node_port(
    client: kube::Client,
    installation_id: &InstallationId,
    workspace: &Workspace,
) -> Result<Option<u16>, kube::Error> {
    let names = WorkspaceRuntimeNames::for_workspace(
        installation_id,
        &workspace.runtime,
        &workspace.short_id,
    )
    .map_err(|error| kube::Error::Service(error.to_string().into()))?;
    let service = Api::<Service>::namespaced(client, workspace.runtime.namespace())
        .get_opt(&names.resources.ssh_service)
        .await?;
    Ok(service.as_ref().and_then(node_port_from_service))
}

pub(super) fn node_port_from_service(service: &Service) -> Option<u16> {
    service
        .spec
        .as_ref()?
        .ports
        .as_ref()?
        .iter()
        .find(|port| port.name.as_deref() == Some("ssh"))?
        .node_port
        .and_then(|port| u16::try_from(port).ok())
}

pub(super) fn restart_generation_is_stale(pod: &Pod, expected: u64) -> bool {
    pod.metadata
        .annotations
        .as_ref()
        .and_then(|annotations| annotations.get("workspace.memeloop.dev/generation"))
        .is_none_or(|generation| generation != &expected.to_string())
}

async fn verify_existing<K>(
    api: &Api<K>,
    name: &str,
    builder: &ResourceBuilder,
    workspace_id: Uuid,
) -> Result<(), ReconcileError>
where
    K: Clone + DeserializeOwned + Debug,
{
    if let Some(existing) = api.get_metadata_opt(name).await? {
        builder.verify_delete_ownership(&existing.metadata, workspace_id)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteProgress {
    DeletionRequested,
    Terminating,
    Gone,
}

#[derive(Debug, Error)]
pub enum ReconcileError {
    #[error(
        "workspace StatefulSet explicitly uses Home PVC {claim_name} without a persisted binding; adopt this claim before reconciling"
    )]
    UnpersistedHomeVolumeBinding { claim_name: String },
    #[error("bound Home PVC {claim_name} is unavailable or invalid: {reason}")]
    InvalidHomeVolumeBinding {
        claim_name: String,
        reason: &'static str,
    },
    #[error("bound scratch PVC {claim_name} is unavailable or its UID/capacity/ownership changed")]
    InvalidScratchVolumeBinding { claim_name: String },
    #[error(transparent)]
    Build(#[from] BuildError),
    #[error(transparent)]
    Materialization(#[from] crate::kubernetes::MaterializationError),
    #[error(transparent)]
    RuntimeIdentity(#[from] WorkspaceRuntimeIdentityError),
    #[error(transparent)]
    Ownership(#[from] OwnershipError),
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),
    #[error(transparent)]
    Kubernetes(#[from] kube::Error),
    #[error("desired Kubernetes object has no metadata.name")]
    MissingObjectName,
    #[error("desired workspace StatefulSet has no Pod template metadata")]
    MissingPodTemplateMetadata,
    #[error("desired workspace StatefulSet has no workspace container")]
    MissingWorkspaceContainer,
    #[error("desired Kubernetes resources do not match the persisted workspace runtime identity")]
    RuntimeIdentityMismatch,
    #[error("ttyd mTLS EnvoyFilter requested without mTLS configuration")]
    MissingTtydMtlsConfig,
    #[error("configured Higress ttyd client Secret is missing")]
    MissingTtydMtlsSecret,
    #[error("configured Higress ttyd client CA companion Secret is missing")]
    MissingTtydMtlsCompanion,
    #[error("configured Higress ttyd client Secret lacks tls.crt or tls.key")]
    InvalidTtydMtlsClientSecret,
    #[error("configured Higress ttyd client CA companion lacks cacert")]
    InvalidTtydMtlsCompanion,
    #[error("owned Higress EnvoyFilter has no UID; refusing an unguarded delete")]
    MissingEnvoyFilterUid,
    #[error("waiting for Web Shell Ingress removal before deleting its TLS validation filter")]
    WebShellIngressTerminating,
    #[error("waiting for an immutable workspace StatefulSet to be recreated")]
    StatefulSetRecreationPending,
    #[error("waiting for temporary scratch PVC removal or concurrent creation")]
    ScratchVolumeRecreationPending,
}
