use crate::workspaces::Workspace;
use k8s_openapi::api::{
    apps::v1::StatefulSet,
    core::v1::{ConfigMap, Namespace, PersistentVolumeClaim, Pod, Secret, Service, ServiceAccount},
    networking::v1::{Ingress, NetworkPolicy},
    rbac::v1::ClusterRoleBinding,
};
use kube::{
    Api,
    api::{DeleteParams, Patch, PatchParams},
};

use super::super::{DesiredResources, InjectionMaterialization};
use super::{
    FIELD_MANAGER, KubernetesCoordinator, ReconcileError, restart_generation_is_stale,
    verify_existing,
};

#[path = "client_apply/port_mappings.rs"]
mod port_mappings;

impl KubernetesCoordinator {
    pub(super) async fn apply_desired(
        &self,
        workspace: &Workspace,
        desired: &DesiredResources,
    ) -> Result<(), ReconcileError> {
        let namespace_name = desired
            .namespace
            .metadata
            .name
            .as_deref()
            .ok_or(ReconcileError::MissingObjectName)?;
        if namespace_name != workspace.runtime.namespace() {
            return Err(ReconcileError::RuntimeIdentityMismatch);
        }
        self.apply_namespace(workspace, desired).await?;
        self.apply_access_identity(namespace_name, workspace, desired)
            .await?;
        self.apply_injections(workspace, &desired.injections)
            .await?;
        self.apply_workspace_identity(namespace_name, workspace, desired)
            .await?;
        // Publish isolation intent before creating any workspace containers. Policy acceptance
        // is not an enforcement acknowledgement; node-level acceptance must verify convergence.
        self.apply_network(namespace_name, workspace, desired)
            .await?;
        self.apply_workload(namespace_name, workspace, desired)
            .await
    }

    async fn apply_namespace(
        &self,
        workspace: &Workspace,
        desired: &DesiredResources,
    ) -> Result<(), ReconcileError> {
        let namespace_name = workspace.runtime.namespace();
        let namespaces = Api::<Namespace>::all(self.client.clone());
        if let Some(existing) = namespaces.get_metadata_opt(namespace_name).await? {
            self.builder
                .verify_installation_ownership(&existing.metadata)?;
            if let Some(actual) = existing
                .metadata
                .labels
                .as_ref()
                .and_then(|labels| labels.get(super::super::WORKSPACE_ID_LABEL))
            {
                return Err(super::super::OwnershipError::LabelMismatch {
                    key: super::super::WORKSPACE_ID_LABEL,
                    expected: "absent from the product namespace".to_owned(),
                    actual: Some(actual.clone()),
                }
                .into());
            }
        }
        namespaces
            .patch(
                namespace_name,
                &PatchParams::apply(FIELD_MANAGER),
                &Patch::Apply(&desired.namespace),
            )
            .await?;
        Ok(())
    }

    async fn apply_access_identity(
        &self,
        namespace_name: &str,
        workspace: &Workspace,
        desired: &DesiredResources,
    ) -> Result<(), ReconcileError> {
        let workspace_id = workspace.id;
        let apply = PatchParams::apply(FIELD_MANAGER);
        let runtime_names = self.builder.runtime_names(workspace)?;
        let names = &runtime_names.resources;
        let service_accounts =
            Api::<ServiceAccount>::namespaced(self.client.clone(), namespace_name);
        let cluster_role_bindings = Api::<ClusterRoleBinding>::all(self.client.clone());
        let binding_name = self.builder.cluster_admin_binding_name(workspace)?;
        if let Some(service_account) = &desired.service_account {
            verify_existing(
                &service_accounts,
                &names.service_account,
                &self.builder,
                workspace_id,
            )
            .await?;
            service_accounts
                .patch(
                    &names.service_account,
                    &apply,
                    &Patch::Apply(service_account),
                )
                .await?;
        }
        if let Some(binding) = &desired.cluster_role_binding {
            verify_existing(
                &cluster_role_bindings,
                &binding_name,
                &self.builder,
                workspace_id,
            )
            .await?;
            cluster_role_bindings
                .patch(&binding_name, &apply, &Patch::Apply(binding))
                .await?;
        } else {
            if let Some(existing) = cluster_role_bindings.get_opt(&binding_name).await? {
                self.builder
                    .verify_delete_ownership(&existing.metadata, workspace_id)?;
                cluster_role_bindings
                    .delete(&binding_name, &DeleteParams::default())
                    .await?;
            }
            if let Some(existing) = service_accounts.get_opt(&names.service_account).await? {
                self.builder
                    .verify_delete_ownership(&existing.metadata, workspace_id)?;
                service_accounts
                    .delete(&names.service_account, &DeleteParams::default())
                    .await?;
            }
        }
        Ok(())
    }

    async fn apply_workspace_identity(
        &self,
        namespace_name: &str,
        workspace: &Workspace,
        desired: &DesiredResources,
    ) -> Result<(), ReconcileError> {
        let workspace_id = workspace.id;
        let apply = PatchParams::apply(FIELD_MANAGER);
        let runtime_names = self.builder.runtime_names(workspace)?;
        let names = &runtime_names.resources;
        let secrets = Api::<Secret>::namespaced(self.client.clone(), namespace_name);
        verify_existing(
            &secrets,
            &names.ssh_identity_secret,
            &self.builder,
            workspace_id,
        )
        .await?;
        secrets
            .patch(
                &names.ssh_identity_secret,
                &apply,
                &Patch::Apply(&desired.ssh_identity),
            )
            .await?;

        let config_maps = Api::<ConfigMap>::namespaced(self.client.clone(), namespace_name);
        verify_existing(
            &config_maps,
            &names.workspace_config,
            &self.builder,
            workspace_id,
        )
        .await?;
        config_maps
            .patch(
                &names.workspace_config,
                &apply,
                &Patch::Apply(&desired.workspace_config),
            )
            .await?;
        Ok(())
    }

    async fn apply_workload(
        &self,
        namespace_name: &str,
        workspace: &Workspace,
        desired: &DesiredResources,
    ) -> Result<(), ReconcileError> {
        let workspace_id = workspace.id;
        let runtime_names = self.builder.runtime_names(workspace)?;
        let names = &runtime_names.resources;
        let apply = PatchParams::apply(FIELD_MANAGER);
        let services = Api::<Service>::namespaced(self.client.clone(), namespace_name);
        verify_existing(&services, &names.service, &self.builder, workspace_id).await?;
        services
            .patch(&names.service, &apply, &Patch::Apply(&desired.service))
            .await?;
        if let Some(service) = &desired.internal_ssh_service {
            verify_existing(&services, &names.ssh_service, &self.builder, workspace_id).await?;
            services
                .patch(&names.ssh_service, &apply, &Patch::Apply(service))
                .await?;
        } else if let Some(existing) = services.get_opt(&names.ssh_service).await? {
            self.builder
                .verify_delete_ownership(&existing.metadata, workspace_id)?;
            services
                .delete(&names.ssh_service, &DeleteParams::default())
                .await?;
        }
        let stateful_sets = Api::<StatefulSet>::namespaced(self.client.clone(), namespace_name);
        verify_existing(
            &stateful_sets,
            &names.stateful_set,
            &self.builder,
            workspace_id,
        )
        .await?;
        stateful_sets
            .patch(
                &names.stateful_set,
                &apply,
                &Patch::Apply(&desired.stateful_set),
            )
            .await?;
        let persistent_volume_claims =
            Api::<PersistentVolumeClaim>::namespaced(self.client.clone(), namespace_name);
        let pvc_name = names.data_pvc_ordinal_zero();
        if let Some(existing) = persistent_volume_claims.get_opt(&pvc_name).await? {
            self.builder
                .verify_delete_ownership(&existing.metadata, workspace_id)?;
            persistent_volume_claims
                .patch(
                    &pvc_name,
                    &PatchParams::default(),
                    &Patch::Merge(&serde_json::json!({
                        "metadata": {"labels": desired.stateful_set.metadata.labels}
                    })),
                )
                .await?;
        }
        if matches!(
            workspace.state,
            crate::workspaces::WorkspaceState::Starting
                | crate::workspaces::WorkspaceState::Restarting
        ) {
            let pods = Api::<Pod>::namespaced(self.client.clone(), namespace_name);
            let pod_name = names.pod_ordinal_zero();
            if let Some(pod) = pods.get_opt(&pod_name).await? {
                self.builder
                    .verify_delete_ownership(&pod.metadata, workspace_id)?;
                if restart_generation_is_stale(&pod, workspace.generation) {
                    pods.delete(&pod_name, &DeleteParams::default()).await?;
                }
            }
        }
        Ok(())
    }

    async fn apply_network(
        &self,
        namespace_name: &str,
        workspace: &Workspace,
        desired: &DesiredResources,
    ) -> Result<(), ReconcileError> {
        let workspace_id = workspace.id;
        let apply = PatchParams::apply(FIELD_MANAGER);
        let runtime_names = self.builder.runtime_names(workspace)?;
        let names = &runtime_names.resources;
        let network_policies =
            Api::<NetworkPolicy>::namespaced(self.client.clone(), namespace_name);
        verify_existing(
            &network_policies,
            &names.network_policy,
            &self.builder,
            workspace_id,
        )
        .await?;
        network_policies
            .patch(
                &names.network_policy,
                &apply,
                &Patch::Apply(&desired.network_policy),
            )
            .await?;
        let ingresses = Api::<Ingress>::namespaced(self.client.clone(), namespace_name);
        if let Some(existing) = ingresses.get_opt(&names.web_shell_ingress).await? {
            self.builder
                .verify_delete_ownership(&existing.metadata, workspace_id)?;
            if desired.web_shell_ingress.is_none() {
                ingresses
                    .delete(&names.web_shell_ingress, &DeleteParams::default())
                    .await?;
            }
        }
        if let Some(ingress) = &desired.web_shell_ingress {
            ingresses
                .patch(&names.web_shell_ingress, &apply, &Patch::Apply(ingress))
                .await?;
        }
        Ok(())
    }

    async fn apply_injections(
        &self,
        workspace: &Workspace,
        materialization: &InjectionMaterialization,
    ) -> Result<(), ReconcileError> {
        let workspace_id = workspace.id;
        let runtime_names = self.builder.runtime_names(workspace)?;
        let names = &runtime_names.resources;
        let namespace = materialization
            .file_config_map
            .metadata
            .namespace
            .as_deref()
            .ok_or(ReconcileError::MissingObjectName)?;
        if namespace != workspace.runtime.namespace() {
            return Err(ReconcileError::RuntimeIdentityMismatch);
        }
        let apply = PatchParams::apply(FIELD_MANAGER);
        let secrets = Api::<Secret>::namespaced(self.client.clone(), namespace);
        verify_existing(
            &secrets,
            &names.environment_secret,
            &self.builder,
            workspace_id,
        )
        .await?;
        secrets
            .patch(
                &names.environment_secret,
                &apply,
                &Patch::Apply(&materialization.environment_secret),
            )
            .await?;
        verify_existing(&secrets, &names.files_secret, &self.builder, workspace_id).await?;
        secrets
            .patch(
                &names.files_secret,
                &apply,
                &Patch::Apply(&materialization.file_secret),
            )
            .await?;
        let config_maps = Api::<ConfigMap>::namespaced(self.client.clone(), namespace);
        verify_existing(
            &config_maps,
            &names.environment_config_map,
            &self.builder,
            workspace_id,
        )
        .await?;
        config_maps
            .patch(
                &names.environment_config_map,
                &apply,
                &Patch::Apply(&materialization.environment_config_map),
            )
            .await?;
        verify_existing(
            &config_maps,
            &names.files_config_map,
            &self.builder,
            workspace_id,
        )
        .await?;
        config_maps
            .patch(
                &names.files_config_map,
                &apply,
                &Patch::Apply(&materialization.file_config_map),
            )
            .await?;
        Ok(())
    }
}
