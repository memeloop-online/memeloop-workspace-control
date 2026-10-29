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
#[path = "client_storage_shrink.rs"]
mod storage_shrink;
pub use home_volume::home_volume_capacity_bytes;
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
            self.has_observed_replicas_inner(workspace, expected),
        )
        .await
    }

    async fn has_observed_replicas_inner(
        &self,
        workspace: &Workspace,
        expected: i32,
    ) -> Result<bool, ReconcileError> {
        let namespace_name = workspace.runtime.namespace();
        let names = self.builder.runtime_names(workspace)?;
        let Some(stateful_set) = apply::observe_runtime_stage(
            workspace,
            "observe_statefulset_read",
            Box::pin(
                Api::<StatefulSet>::namespaced(self.client.clone(), namespace_name)
                    .get_opt(&names.resources.stateful_set),
            ),
        )
        .await?
        else {
            return Ok(false);
        };
        self.builder
            .verify_delete_ownership(&stateful_set.metadata, workspace.id)?;
        let Some(spec) = stateful_set.spec.as_ref() else {
            return Ok(false);
        };
        let Some(status) = stateful_set.status.as_ref() else {
            return Ok(false);
        };
        if spec.replicas != Some(expected)
            || stateful_set.metadata.deletion_timestamp.is_some()
            || stateful_set.metadata.generation.is_none()
            || status.observed_generation.is_none()
            || status.observed_generation < stateful_set.metadata.generation
            || spec.template.metadata.as_ref().is_none_or(|metadata| {
                metadata.annotations.as_ref().is_none_or(|annotations| {
                    annotations.get("workspace.memeloop.dev/generation")
                        != Some(&workspace.generation.to_string())
                })
            })
        {
            return Ok(false);
        }
        if expected == 0 {
            return Ok(status.replicas == 0);
        }
        // StatefulSet aggregates may still describe a deleted Pod. The current
        // Pod is authoritative in both directions, including Pending and absent.
        let Some(revision) = status.update_revision.as_deref() else {
            return Ok(false);
        };
        let Some(uid) = stateful_set.metadata.uid.as_deref() else {
            return Ok(false);
        };
        if expected != 1 {
            return Ok(false);
        }
        let Some(pod) = apply::observe_runtime_stage(
            workspace,
            "observe_ready_pod_read",
            Box::pin(
                Api::<Pod>::namespaced(self.client.clone(), namespace_name)
                    .get_opt(&names.resources.pod_ordinal_zero()),
            ),
        )
        .await?
        else {
            return Ok(false);
        };
        self.builder
            .verify_delete_ownership(&pod.metadata, workspace.id)?;
        Ok(pod.metadata.uid.is_some()
            && pod.metadata.deletion_timestamp.is_none()
            && !restart_generation_is_stale(&pod, workspace.generation)
            && pod
                .metadata
                .owner_references
                .as_ref()
                .is_some_and(|owners| {
                    owners.iter().any(|owner| {
                        owner.controller == Some(true)
                            && owner.api_version == "apps/v1"
                            && owner.kind == "StatefulSet"
                            && owner.name == names.resources.stateful_set
                            && owner.uid == uid
                    })
                })
            && pod.metadata.labels.as_ref().is_some_and(|labels| {
                labels.get("controller-revision-hash").map(String::as_str) == Some(revision)
            })
            && pod.status.as_ref().is_some_and(|status| {
                status.phase.as_deref() == Some("Running")
                    && status.conditions.as_ref().is_some_and(|conditions| {
                        conditions.iter().any(|condition| {
                            condition.type_ == "Ready" && condition.status == "True"
                        })
                    })
            }))
    }
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
