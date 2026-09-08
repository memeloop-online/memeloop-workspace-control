use std::fmt::Debug;

use k8s_openapi::api::{
    apps::v1::StatefulSet,
    core::v1::{ConfigMap, Namespace, PersistentVolumeClaim, Pod, Secret, Service, ServiceAccount},
    networking::v1::{Ingress, NetworkPolicy},
    rbac::v1::ClusterRoleBinding,
};
use kube::{
    Api, Resource, ResourceExt,
    api::{DeleteParams, ListParams, Preconditions},
    core::DynamicObject,
};
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::{workspace_runtime::WorkspaceResourceNames, workspaces::Workspace};

use super::{DeleteProgress, KubernetesCoordinator, ReconcileError, ResourceBuilder};

impl KubernetesCoordinator {
    /// Starts deletion or confirms that Kubernetes has removed every owned workspace resource.
    /// The caller must only mark the database row deleted after receiving `Gone`.
    pub async fn delete_or_confirm(
        &self,
        workspace: &Workspace,
    ) -> Result<DeleteProgress, ReconcileError> {
        let workspace_id = workspace.id;
        let namespace_name = workspace.runtime.namespace();
        let runtime_names = self.builder.runtime_names(workspace)?;
        let names = &runtime_names.resources;
        let binding_name = self.builder.cluster_admin_binding_name(workspace)?;
        let cluster_role_bindings = Api::<ClusterRoleBinding>::all(self.client.clone());
        if delete_owned_if_present(
            &cluster_role_bindings,
            &binding_name,
            &self.builder,
            workspace_id,
        )
        .await?
        {
            return Ok(DeleteProgress::DeletionRequested);
        }

        let namespaces = Api::<Namespace>::all(self.client.clone());
        let Some(namespace) = namespaces.get_opt(namespace_name).await? else {
            return Ok(DeleteProgress::Gone);
        };
        self.builder
            .verify_installation_ownership(&namespace.metadata)?;
        self.delete_workspace_resources(workspace, names).await
    }

    async fn delete_workspace_resources(
        &self,
        workspace: &Workspace,
        names: &WorkspaceResourceNames,
    ) -> Result<DeleteProgress, ReconcileError> {
        let namespace = workspace.runtime.namespace();
        let workspace_id = workspace.id;
        let mapping_resources = ListParams::default().labels(&format!(
            "{}={},{}={},{}",
            super::super::OWNER_INSTALLATION_LABEL,
            self.builder.installation_id,
            super::super::WORKSPACE_ID_LABEL,
            workspace_id,
            super::super::port_mappings::PORT_MAPPING_ID_LABEL,
        ));

        // Delete externally reachable resources first. Mapping objects are
        // selected by both installation and workspace ownership labels so a
        // sibling in the same namespace can never be swept up.
        let ingresses = Api::<Ingress>::namespaced(self.client.clone(), namespace);
        if delete_owned_if_present(
            &ingresses,
            &names.web_shell_ingress,
            &self.builder,
            workspace_id,
        )
        .await?
            || delete_first_owned(&ingresses, &mapping_resources, &self.builder, workspace_id)
                .await?
        {
            return Ok(DeleteProgress::DeletionRequested);
        }
        if self.builder.ttyd_mtls.is_some() {
            let envoy_filters = Api::<DynamicObject>::namespaced_with(
                self.client.clone(),
                &self.builder.higress_namespace,
                &super::super::envoy_filter::api_resource(),
            );
            if let Some(existing) = envoy_filters.get_opt(&names.web_shell_envoy_filter).await? {
                self.builder
                    .verify_delete_ownership(&existing.metadata, workspace_id)?;
                let uid = existing
                    .metadata
                    .uid
                    .ok_or(super::ReconcileError::MissingEnvoyFilterUid)?;
                envoy_filters
                    .delete(
                        &names.web_shell_envoy_filter,
                        &DeleteParams::default().preconditions(Preconditions {
                            uid: Some(uid),
                            resource_version: None,
                        }),
                    )
                    .await?;
                return Ok(DeleteProgress::DeletionRequested);
            }
        }
        let mapping_policies = Api::<NetworkPolicy>::namespaced(self.client.clone(), namespace);
        if delete_first_owned(
            &mapping_policies,
            &mapping_resources,
            &self.builder,
            workspace_id,
        )
        .await?
        {
            return Ok(DeleteProgress::DeletionRequested);
        }
        let services = Api::<Service>::namespaced(self.client.clone(), namespace);
        if delete_first_owned(&services, &mapping_resources, &self.builder, workspace_id).await?
            || delete_owned_if_present(&services, &names.ssh_service, &self.builder, workspace_id)
                .await?
            || delete_owned_if_present(&services, &names.service, &self.builder, workspace_id)
                .await?
        {
            return Ok(DeleteProgress::DeletionRequested);
        }

        let stateful_sets = Api::<StatefulSet>::namespaced(self.client.clone(), namespace);
        if delete_owned_if_present(
            &stateful_sets,
            &names.stateful_set,
            &self.builder,
            workspace_id,
        )
        .await?
        {
            return Ok(DeleteProgress::DeletionRequested);
        }
        if self
            .workspace_pod_or_pvc_reference_is_present(namespace, names, workspace_id)
            .await?
        {
            return Ok(DeleteProgress::Terminating);
        }
        self.delete_workspace_remaining(namespace, names, workspace_id, &mapping_policies)
            .await
    }

    async fn workspace_pod_or_pvc_reference_is_present(
        &self,
        namespace: &str,
        names: &WorkspaceResourceNames,
        workspace_id: Uuid,
    ) -> Result<bool, ReconcileError> {
        let pods = Api::<Pod>::namespaced(self.client.clone(), namespace);
        let target_pod = names.pod_ordinal_zero();
        if let Some(pod) = pods.get_opt(&target_pod).await? {
            self.builder
                .verify_delete_ownership(pod.meta(), workspace_id)?;
            // StatefulSet deletion uses Kubernetes background propagation. Do
            // not remove its durable claim or report Gone until the owned Pod
            // has actually disappeared.
            return Ok(true);
        }
        let data_pvc = names.data_pvc_ordinal_zero();
        for pod in pods.list(&ListParams::default()).await?.items {
            if pod.name_any() == target_pod {
                self.builder
                    .verify_delete_ownership(pod.meta(), workspace_id)?;
                return Ok(true);
            }
            if pod_references_pvc(&pod, &data_pvc) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn delete_workspace_remaining(
        &self,
        namespace: &str,
        names: &WorkspaceResourceNames,
        workspace_id: Uuid,
        mapping_policies: &Api<NetworkPolicy>,
    ) -> Result<DeleteProgress, ReconcileError> {
        if delete_owned_if_present(
            mapping_policies,
            &names.network_policy,
            &self.builder,
            workspace_id,
        )
        .await?
        {
            return Ok(DeleteProgress::DeletionRequested);
        }
        let config_maps = Api::<ConfigMap>::namespaced(self.client.clone(), namespace);
        for name in [
            &names.workspace_config,
            &names.environment_config_map,
            &names.files_config_map,
        ] {
            if delete_owned_if_present(&config_maps, name, &self.builder, workspace_id).await? {
                return Ok(DeleteProgress::DeletionRequested);
            }
        }
        let secrets = Api::<Secret>::namespaced(self.client.clone(), namespace);
        for name in [
            &names.ssh_identity_secret,
            &names.environment_secret,
            &names.files_secret,
        ] {
            if delete_owned_if_present(&secrets, name, &self.builder, workspace_id).await? {
                return Ok(DeleteProgress::DeletionRequested);
            }
        }
        let service_accounts = Api::<ServiceAccount>::namespaced(self.client.clone(), namespace);
        if delete_owned_if_present(
            &service_accounts,
            &names.service_account,
            &self.builder,
            workspace_id,
        )
        .await?
        {
            return Ok(DeleteProgress::DeletionRequested);
        }
        let pvcs = Api::<PersistentVolumeClaim>::namespaced(self.client.clone(), namespace);
        if delete_owned_if_present(
            &pvcs,
            &names.data_pvc_ordinal_zero(),
            &self.builder,
            workspace_id,
        )
        .await?
        {
            return Ok(DeleteProgress::DeletionRequested);
        }
        Ok(DeleteProgress::Gone)
    }
}

fn pod_references_pvc(pod: &Pod, pvc_name: &str) -> bool {
    pod.spec
        .as_ref()
        .and_then(|spec| spec.volumes.as_ref())
        .is_some_and(|volumes| {
            volumes.iter().any(|volume| {
                volume
                    .persistent_volume_claim
                    .as_ref()
                    .is_some_and(|claim| claim.claim_name == pvc_name)
            })
        })
}

async fn delete_owned_if_present<K>(
    api: &Api<K>,
    name: &str,
    builder: &ResourceBuilder,
    workspace_id: Uuid,
) -> Result<bool, ReconcileError>
where
    K: Clone + DeserializeOwned + Debug + Resource<DynamicType = ()>,
{
    let Some(existing) = api.get_opt(name).await? else {
        return Ok(false);
    };
    builder.verify_delete_ownership(existing.meta(), workspace_id)?;
    api.delete(name, &DeleteParams::default()).await?;
    Ok(true)
}

async fn delete_first_owned<K>(
    api: &Api<K>,
    list: &ListParams,
    builder: &ResourceBuilder,
    workspace_id: Uuid,
) -> Result<bool, ReconcileError>
where
    K: Clone + DeserializeOwned + Debug + Resource<DynamicType = ()>,
{
    let Some(existing) = api.list(list).await?.into_iter().next() else {
        return Ok(false);
    };
    builder.verify_delete_ownership(existing.meta(), workspace_id)?;
    let name = existing.name_any();
    api.delete(&name, &DeleteParams::default()).await?;
    Ok(true)
}
