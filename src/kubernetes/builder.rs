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
use kube::core::DynamicObject;
use thiserror::Error;
use uuid::Uuid;

use super::workspace_pod::WorkspacePod;
use super::{
    InternetEgressConfig, MaterializationError, OwnershipError,
    materialization::InjectionMaterialization,
};
use super::{
    MANAGED_BY_LABEL, ORGANIZATION_ID_LABEL, OWNER_INSTALLATION_LABEL, OWNER_USER_ID_LABEL,
    ResolvedPlacement, TEMPLATE_ID_LABEL, TtydMtlsConfig, WORKSPACE_ID_LABEL, envoy_filter,
    higress, http_proxy, materialization, network_policy, ownership, port_mappings,
    resource_helpers, workload,
};
use crate::{
    config::InstallationId,
    workspace_runtime::{
        WorkspaceRuntimeIdentity, WorkspaceRuntimeIdentityError, WorkspaceRuntimeNames,
    },
    workspaces::{Workspace, WorkspaceState},
};
use resource_helpers::{
    cluster_admin_binding, cluster_admin_service_account, internal_ssh_service, pod_labels,
    service, workspace_config,
};

#[derive(Debug, Clone)]
pub struct ResourceBuilder {
    pub installation_id: InstallationId,
    pub ttyd_image: String,
    pub buildkit_image: String,
    pub ttyd_mtls: Option<TtydMtlsConfig>,
    pub higress_namespace: String,
    pub higress_pod_labels: BTreeMap<String, String>,
    pub higress_source_cidrs: Vec<String>,
    pub internet_egress: Option<InternetEgressConfig>,
    pub jump_host_namespace: String,
    pub jump_host_pod_labels: BTreeMap<String, String>,
    /// StorageClass for the durable workspace Home PVC.
    pub storage_class_name: Option<String>,
    /// Optional local CSI class for Pod-owned generic ephemeral scratch PVCs.
    pub scratch_storage_class_name: Option<String>,
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
    pub web_shell_envoy_filter: Option<DynamicObject>,
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
        self.build_with_placement(workspace, &ResolvedPlacement::default())
    }

    pub fn build_with_placement(
        &self,
        workspace: &Workspace,
        placement: &ResolvedPlacement,
    ) -> Result<DesiredResources, BuildError> {
        if matches!(
            workspace.state,
            WorkspaceState::Deleting | WorkspaceState::Deleted
        ) {
            return Err(BuildError::WorkspaceBeingDeleted);
        }
        if workspace.template.image.trim().is_empty() {
            return Err(BuildError::EmptyImage);
        }
        if workspace.template.egress_policy == crate::templates::EgressPolicy::InternetOnly
            && self.internet_egress.is_none()
        {
            return Err(BuildError::InternetEgressNotConfigured);
        }
        self.validate_ttyd_gateway()?;

        workspace.runtime.validate_for_workspace(
            &self.installation_id,
            workspace.id,
            &workspace.short_id,
        )?;
        let names = self.runtime_names(workspace)?;
        let stable_labels = self.labels(workspace.id);
        let labels = self.workspace_labels(workspace);
        let namespace_labels = self.installation_labels();
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
        let (web_shell_ingress, web_shell_envoy_filter) = self.web_shell_resources(&names, &labels);
        Ok(DesiredResources {
            namespace: Namespace {
                metadata: ObjectMeta {
                    name: Some(workspace.runtime.namespace().to_owned()),
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
            stateful_set: self.stateful_set(
                &names,
                &labels,
                &pod_labels,
                workspace,
                placement,
                replicas,
            ),
            network_policy: network_policy::build(
                &names,
                &labels,
                &selector_labels,
                &self.higress_namespace,
                &self.higress_pod_labels,
                &self.higress_source_cidrs,
                self.internet_egress.as_ref(),
                &self.jump_host_namespace,
                &self.jump_host_pod_labels,
                workspace.template.access_mode,
                workspace.template.egress_policy,
                self.internal_ssh_node_port_enabled,
            ),
            injections,
            workspace_config: workspace_config(
                &names,
                &labels,
                WorkspacePod::from_template(&workspace.template),
            ),
            ssh_identity: resource_helpers::ssh_identity(&names, &labels, None),
            web_shell_ingress,
            web_shell_envoy_filter,
        })
    }

    fn web_shell_resources(
        &self,
        names: &WorkspaceRuntimeNames,
        labels: &BTreeMap<String, String>,
    ) -> (Option<Ingress>, Option<DynamicObject>) {
        let ingress = self.web_shell_domain.as_ref().map(|domain| {
            higress::web_shell_ingress(names, labels, domain, self.ttyd_mtls.as_ref())
        });
        let filter = self.web_shell_domain.as_ref().and_then(|_| {
            self.ttyd_mtls.as_ref().map(|mtls| {
                envoy_filter::web_shell_san_filter(
                    names,
                    labels,
                    &self.higress_namespace,
                    &self.higress_pod_labels,
                    mtls,
                )
            })
        });
        (ingress, filter)
    }

    fn validate_ttyd_gateway(&self) -> Result<(), BuildError> {
        if let Some(mtls) = &self.ttyd_mtls {
            if mtls.higress_client_secret_namespace != self.higress_namespace {
                return Err(BuildError::TtydMtlsGatewayNamespaceMismatch);
            }
            if self.higress_pod_labels.is_empty() {
                return Err(BuildError::TtydMtlsGatewaySelectorMissing);
            }
        }
        Ok(())
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
        let (mut service, mut ingress) =
            port_mappings::resources(&names, &labels, &selector_labels, domain, mapping);
        let mut network_policy = port_mappings::network_policy(
            &names,
            &labels,
            &selector_labels,
            &self.higress_namespace,
            &self.higress_pod_labels,
            &self.higress_source_cidrs,
            mapping,
        );
        if let Some(mtls) = &self.ttyd_mtls {
            http_proxy::secure_mapping(
                &mut service,
                &mut ingress,
                &mut network_policy,
                &names,
                mtls,
            );
        }
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
        if let Some(template_id) = workspace.template_id {
            labels.insert(TEMPLATE_ID_LABEL.to_owned(), template_id.to_string());
        }
        labels
    }

    fn stateful_set(
        &self,
        runtime: &WorkspaceRuntimeNames,
        labels: &BTreeMap<String, String>,
        template_labels: &BTreeMap<String, String>,
        workspace: &Workspace,
        placement: &ResolvedPlacement,
        replicas: i32,
    ) -> StatefulSet {
        workload::stateful_set(
            self,
            runtime,
            labels,
            template_labels,
            workspace,
            placement,
            replicas,
        )
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
    #[error("internet_only egress requires configured DNS namespace and Pod labels")]
    InternetEgressNotConfigured,
    #[error("ttyd mTLS client Secret namespace must equal the configured Higress namespace")]
    TtydMtlsGatewayNamespaceMismatch,
    #[error("ttyd mTLS requires non-empty configured Higress gateway Pod labels")]
    TtydMtlsGatewaySelectorMissing,
}
