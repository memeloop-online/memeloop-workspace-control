use std::collections::BTreeMap;

use k8s_openapi::{
    api::{
        apps::v1::StatefulSet,
        core::v1::{ConfigMap, Namespace, Service, ServiceAccount},
        networking::v1::{Ingress, NetworkPolicy},
        rbac::v1::ClusterRoleBinding,
    },
    apimachinery::pkg::apis::meta::v1::ObjectMeta,
};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    config::InstallationId,
    workspace_runtime::{
        WorkspaceNamespaceScope, WorkspaceRuntimeIdentity, WorkspaceRuntimeIdentityError,
        WorkspaceRuntimeNames,
    },
    workspaces::{Workspace, WorkspaceState},
};

mod buildkit;
mod client;
#[cfg(test)]
mod client_tests;
mod higress;
mod materialization;
mod network_policy;
mod ownership;
mod port_mappings;
mod resource_helpers;
mod workload;
mod workspace_pod;

pub use client::{DeleteProgress, KubernetesCoordinator, ReconcileError, workspace_ssh_node_port};
pub use materialization::{InjectionMaterialization, MaterializationError};
pub use ownership::OwnershipError;

pub(crate) use resource_helpers::namespaced_metadata;
use resource_helpers::{
    cluster_admin_binding, cluster_admin_service_account, internal_ssh_service, pod_labels,
    service, workspace_config,
};
use workspace_pod::WorkspacePod;

pub const OWNER_INSTALLATION_LABEL: &str = "workspace.memeloop.dev/owner-installation";
pub const WORKSPACE_ID_LABEL: &str = "workspace.memeloop.dev/workspace-id";
pub const ORGANIZATION_ID_LABEL: &str = "workspace.memeloop.dev/organization-id";
pub const OWNER_USER_ID_LABEL: &str = "workspace.memeloop.dev/owner-user-id";
const COMPONENT_LABEL: &str = "app.kubernetes.io/component";
const MANAGED_BY_LABEL: &str = "app.kubernetes.io/managed-by";

#[derive(Debug, Clone)]
pub struct ResourceBuilder {
    pub installation_id: InstallationId,
    pub ttyd_image: String,
    pub higress_namespace: String,
    pub higress_pod_labels: BTreeMap<String, String>,
    pub higress_source_cidrs: Vec<String>,
    pub jump_host_namespace: String,
    pub jump_host_pod_labels: BTreeMap<String, String>,
    pub storage_class_name: Option<String>,
    pub web_shell_domain: Option<String>,
    pub port_mapping_domain: Option<String>,
    pub higress_gateway_name: String,
    pub higress_https_section_name: String,
    pub internal_ssh_node_port_enabled: bool,
}

#[derive(Debug)]
pub struct DesiredResources {
    pub namespace: Namespace,
    pub service: Service,
    pub internal_ssh_service: Option<Service>,
    pub service_account: Option<ServiceAccount>,
    pub cluster_role_binding: Option<ClusterRoleBinding>,
    pub stateful_set: StatefulSet,
    pub network_policy: NetworkPolicy,
    pub injections: InjectionMaterialization,
    pub workspace_config: ConfigMap,
    pub ssh_identity: k8s_openapi::api::core::v1::Secret,
    pub web_shell_ingress: Option<Ingress>,
}

impl ResourceBuilder {
    pub(crate) fn runtime_names(
        &self,
        workspace: &Workspace,
    ) -> Result<WorkspaceRuntimeNames, WorkspaceRuntimeIdentityError> {
        WorkspaceRuntimeNames::for_workspace(
            &self.installation_id,
            &workspace.runtime,
            &workspace.short_id,
        )
    }

    pub fn build(&self, workspace: &Workspace) -> Result<DesiredResources, BuildError> {
        if matches!(
            workspace.state,
            WorkspaceState::Deleting | WorkspaceState::Deleted
        ) {
            return Err(BuildError::WorkspaceBeingDeleted);
        }
        if workspace.template.image.trim().is_empty() {
            return Err(BuildError::EmptyImage);
        }

        workspace.runtime.validate_for_workspace(
            &self.installation_id,
            workspace.id,
            &workspace.short_id,
        )?;
        let runtime = &workspace.runtime;
        let names = self.runtime_names(workspace)?;
        let stable_labels = self.labels(workspace.id);
        let labels = self.workspace_labels(workspace);
        let namespace_labels = match runtime.namespace_scope {
            WorkspaceNamespaceScope::Dedicated => labels.clone(),
            WorkspaceNamespaceScope::Shared => self.installation_labels(),
        };
        // StatefulSet selectors and volumeClaimTemplates are immutable. Keep those
        // labels limited to the original ownership identity so an upgrade can add
        // observability labels to existing workspaces without replacing storage.
        let selector_labels = pod_labels(&stable_labels);
        let pod_labels = pod_labels(&labels);
        let cluster_access = workspace.template.cluster_access;
        let cluster_admin_binding_name = names.cluster_admin_binding_name(&self.installation_id);
        let replicas = match workspace.state {
            WorkspaceState::Stopping | WorkspaceState::Stopped | WorkspaceState::Failed => 0,
            WorkspaceState::Provisioning
            | WorkspaceState::Ready
            | WorkspaceState::Starting
            | WorkspaceState::Restarting => 1,
            WorkspaceState::Deleting | WorkspaceState::Deleted => unreachable!(),
        };

        let injections = materialization::build(&names, &labels, &[])?;
        Ok(DesiredResources {
            namespace: Namespace {
                metadata: ObjectMeta {
                    name: Some(runtime.namespace.clone()),
                    labels: Some(namespace_labels),
                    ..ObjectMeta::default()
                },
                ..Namespace::default()
            },
            service: service(&names, &labels, &selector_labels),
            internal_ssh_service: internal_ssh_service(
                &names,
                &labels,
                &selector_labels,
                workspace.template.access_mode,
                self.internal_ssh_node_port_enabled,
            ),
            service_account: cluster_admin_service_account(&names, &labels, cluster_access),
            cluster_role_binding: cluster_admin_binding(
                &cluster_admin_binding_name,
                &names,
                &labels,
                cluster_access,
            ),
            stateful_set: self.stateful_set(&names, &labels, &pod_labels, workspace, replicas),
            network_policy: network_policy::build(
                &names,
                &labels,
                &selector_labels,
                &self.higress_namespace,
                &self.higress_pod_labels,
                &self.higress_source_cidrs,
                &self.jump_host_namespace,
                &self.jump_host_pod_labels,
                workspace.template.access_mode,
                self.internal_ssh_node_port_enabled,
            ),
            injections,
            workspace_config: workspace_config(
                &names,
                &labels,
                WorkspacePod::from_template(&workspace.template),
            ),
            ssh_identity: resource_helpers::ssh_identity(&names, &labels, None),
            web_shell_ingress: self
                .web_shell_domain
                .as_ref()
                .map(|domain| higress::web_shell_ingress(&names, &labels, domain)),
        })
    }

    pub fn verify_delete_ownership(
        &self,
        metadata: &ObjectMeta,
        workspace_id: Uuid,
    ) -> Result<(), OwnershipError> {
        ownership::verify(
            metadata,
            self.installation_id.as_str(),
            &workspace_id.to_string(),
        )
    }

    pub fn verify_installation_ownership(
        &self,
        metadata: &ObjectMeta,
    ) -> Result<(), OwnershipError> {
        ownership::verify_installation(metadata, self.installation_id.as_str())
    }

    pub(crate) fn cluster_admin_binding_name(
        &self,
        workspace: &Workspace,
    ) -> Result<String, WorkspaceRuntimeIdentityError> {
        Ok(self
            .runtime_names(workspace)?
            .cluster_admin_binding_name(&self.installation_id))
    }

    pub fn materialize_injections(
        &self,
        workspace_id: Uuid,
        workspace_short_id: &str,
        runtime: &WorkspaceRuntimeIdentity,
        resolved: &[crate::injections::ResolvedInjection],
    ) -> Result<InjectionMaterialization, MaterializationError> {
        let names = WorkspaceRuntimeNames::for_workspace(
            &self.installation_id,
            runtime,
            workspace_short_id,
        )?;
        materialization::build(&names, &self.labels(workspace_id), resolved)
    }

    pub fn materialize_ssh_identity(
        &self,
        workspace_id: Uuid,
        workspace_short_id: &str,
        runtime: &WorkspaceRuntimeIdentity,
        identity: &crate::storage::WorkspaceSshIdentity,
    ) -> Result<k8s_openapi::api::core::v1::Secret, WorkspaceRuntimeIdentityError> {
        let names = WorkspaceRuntimeNames::for_workspace(
            &self.installation_id,
            runtime,
            workspace_short_id,
        )?;
        Ok(resource_helpers::ssh_identity(
            &names,
            &self.labels(workspace_id),
            Some(identity),
        ))
    }

    pub(crate) fn port_mapping_resources(
        &self,
        workspace: &Workspace,
        mapping: &crate::storage::PortMapping,
    ) -> Result<Option<(Service, Ingress, NetworkPolicy)>, BuildError> {
        let Some(domain) = self.port_mapping_domain.as_deref() else {
            return Ok(None);
        };
        workspace.runtime.validate_for_workspace(
            &self.installation_id,
            workspace.id,
            &workspace.short_id,
        )?;
        let names = self.runtime_names(workspace)?;
        let labels = self.workspace_labels(workspace);
        let selector_labels = pod_labels(&self.labels(workspace.id));
        let (service, ingress) =
            port_mappings::resources(&names, &labels, &selector_labels, domain, mapping);
        let network_policy = port_mappings::network_policy(
            &names,
            &labels,
            &selector_labels,
            &self.higress_namespace,
            &self.higress_pod_labels,
            &self.higress_source_cidrs,
            mapping,
        );
        Ok(Some((service, ingress, network_policy)))
    }

    fn labels(&self, workspace_id: Uuid) -> BTreeMap<String, String> {
        let mut labels = self.installation_labels();
        labels.insert(WORKSPACE_ID_LABEL.to_owned(), workspace_id.to_string());
        labels
    }

    fn installation_labels(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            (
                OWNER_INSTALLATION_LABEL.to_owned(),
                self.installation_id.to_string(),
            ),
            (
                MANAGED_BY_LABEL.to_owned(),
                "memeloop-workspace-control".to_owned(),
            ),
        ])
    }

    fn workspace_labels(&self, workspace: &Workspace) -> BTreeMap<String, String> {
        let mut labels = self.labels(workspace.id);
        labels.insert(
            ORGANIZATION_ID_LABEL.to_owned(),
            workspace.organization_id.to_string(),
        );
        labels.insert(
            OWNER_USER_ID_LABEL.to_owned(),
            workspace.owner_id.to_string(),
        );
        labels
    }

    fn stateful_set(
        &self,
        runtime: &WorkspaceRuntimeNames,
        labels: &BTreeMap<String, String>,
        template_labels: &BTreeMap<String, String>,
        workspace: &Workspace,
        replicas: i32,
    ) -> StatefulSet {
        workload::stateful_set(self, runtime, labels, template_labels, workspace, replicas)
    }
}

#[derive(Debug, Error)]
pub enum BuildError {
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),
    #[error(transparent)]
    Materialization(#[from] MaterializationError),
    #[error(transparent)]
    RuntimeIdentity(#[from] WorkspaceRuntimeIdentityError),
    #[error("cannot build desired runtime resources while workspace is being deleted")]
    WorkspaceBeingDeleted,
    #[error("workspace image must not be empty")]
    EmptyImage,
}
