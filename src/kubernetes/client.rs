use std::fmt::Debug;

use k8s_openapi::api::{
    apps::v1::StatefulSet,
    core::v1::{Pod, Secret, Service},
};
use kube::{Api, Client};
use serde::de::DeserializeOwned;
use thiserror::Error;
use uuid::Uuid;

use super::{BuildError, InjectionMaterialization, OwnershipError, ResourceBuilder};
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

#[derive(Clone)]
pub struct KubernetesCoordinator {
    client: Client,
    builder: ResourceBuilder,
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
        injections: InjectionMaterialization,
        ssh_identity: Secret,
    ) -> Result<(), ReconcileError> {
        let mut desired = self.builder.build(workspace)?;
        let revision = injections.revision()?;
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
        let template_metadata = desired
            .stateful_set
            .spec
            .as_mut()
            .and_then(|spec| spec.template.metadata.as_mut())
            .ok_or(ReconcileError::MissingPodTemplateMetadata)?;
        template_metadata
            .annotations
            .get_or_insert_with(Default::default)
            .insert(
                "workspace.memeloop.dev/injection-revision".to_owned(),
                revision,
            );
        self.apply_desired(workspace, &desired).await
    }

    pub async fn has_observed_replicas(
        &self,
        workspace: &Workspace,
        expected: i32,
    ) -> Result<bool, ReconcileError> {
        let namespace_name = workspace.runtime.namespace();
        let names = self.builder.runtime_names(workspace)?;
        let Some(stateful_set) =
            Api::<StatefulSet>::namespaced(self.client.clone(), namespace_name)
                .get_opt(&names.resources.stateful_set)
                .await?
        else {
            return Ok(false);
        };
        let Some(status) = stateful_set.status else {
            return Ok(false);
        };
        if status.observed_generation.unwrap_or_default()
            < stateful_set.metadata.generation.unwrap_or_default()
        {
            return Ok(false);
        }
        if expected == 0 {
            return Ok(status.replicas == 0);
        }
        Ok(status.replicas == expected
            && status.ready_replicas.unwrap_or_default() == expected
            && status.updated_replicas.unwrap_or_default() == expected
            && status.current_revision.is_some()
            && status.current_revision == status.update_revision)
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
}
