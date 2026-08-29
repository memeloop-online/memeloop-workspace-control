use std::fmt::Debug;

use k8s_openapi::api::{
    apps::v1::StatefulSet,
    core::v1::{Namespace, Pod, Secret, Service, ServiceAccount},
    networking::v1::Ingress,
    rbac::v1::ClusterRoleBinding,
};
use kube::{Api, Client, api::DeleteParams};
use serde::de::DeserializeOwned;
use thiserror::Error;
use uuid::Uuid;

use super::{BuildError, InjectionMaterialization, OwnershipError, ResourceBuilder};
use crate::workspaces::Workspace;

const FIELD_MANAGER: &str = "memeloop-workspace-control";

#[path = "client_apply.rs"]
mod apply;

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
        workspace_short_id: &str,
        expected: i32,
    ) -> Result<bool, ReconcileError> {
        let namespace_name = self
            .builder
            .installation_id
            .workspace_namespace(workspace_short_id)?;
        let Some(stateful_set) =
            Api::<StatefulSet>::namespaced(self.client.clone(), &namespace_name)
                .get_opt("workspace")
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

    /// Starts deletion or confirms that Kubernetes has finished removing the namespace.
    /// The caller must only mark the database row deleted after receiving `Gone`.
    pub async fn delete_or_confirm(
        &self,
        workspace_id: Uuid,
        workspace_short_id: &str,
    ) -> Result<DeleteProgress, ReconcileError> {
        let namespace_name = self
            .builder
            .installation_id
            .workspace_namespace(workspace_short_id)?;
        let binding_name = self.builder.cluster_admin_binding_name(workspace_short_id);
        let cluster_role_bindings = Api::<ClusterRoleBinding>::all(self.client.clone());
        if let Some(binding) = cluster_role_bindings.get_opt(&binding_name).await? {
            self.builder
                .verify_delete_ownership(&binding.metadata, workspace_id)?;
            cluster_role_bindings
                .delete(&binding_name, &DeleteParams::default())
                .await?;
            return Ok(DeleteProgress::DeletionRequested);
        }
        let namespaces = Api::<Namespace>::all(self.client.clone());
        let Some(namespace) = namespaces.get_opt(&namespace_name).await? else {
            return Ok(DeleteProgress::Gone);
        };

        self.builder
            .verify_delete_ownership(&namespace.metadata, workspace_id)?;
        let service_accounts =
            Api::<ServiceAccount>::namespaced(self.client.clone(), &namespace_name);
        if let Some(service_account) = service_accounts.get_opt("workspace-admin").await? {
            self.builder
                .verify_delete_ownership(&service_account.metadata, workspace_id)?;
            service_accounts
                .delete("workspace-admin", &DeleteParams::default())
                .await?;
            return Ok(DeleteProgress::DeletionRequested);
        }
        let ingresses = Api::<Ingress>::namespaced(self.client.clone(), &namespace_name);
        if let Some(ingress) = ingresses.get_opt("web-shell").await? {
            self.builder
                .verify_delete_ownership(&ingress.metadata, workspace_id)?;
            ingresses
                .delete("web-shell", &DeleteParams::default())
                .await?;
            // Keep deletion deliberately staged: do not start Namespace removal
            // until the externally visible Web Shell route is confirmed gone.
            return Ok(DeleteProgress::DeletionRequested);
        }
        if namespace.metadata.deletion_timestamp.is_some() {
            return Ok(DeleteProgress::Terminating);
        }
        namespaces
            .delete(&namespace_name, &DeleteParams::default())
            .await?;
        Ok(DeleteProgress::DeletionRequested)
    }
}

/// Returns the apiserver-assigned SSH NodePort for an internal workspace.
pub async fn workspace_ssh_node_port(
    client: kube::Client,
    namespace: &str,
) -> Result<Option<u16>, kube::Error> {
    let service = Api::<Service>::namespaced(client, namespace)
        .get_opt("workspace-ssh")
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
    Ownership(#[from] OwnershipError),
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),
    #[error(transparent)]
    Kubernetes(#[from] kube::Error),
    #[error("desired Kubernetes object has no metadata.name")]
    MissingObjectName,
    #[error("desired workspace StatefulSet has no Pod template metadata")]
    MissingPodTemplateMetadata,
}
