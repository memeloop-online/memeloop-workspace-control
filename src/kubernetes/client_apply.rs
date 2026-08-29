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
use uuid::Uuid;

use crate::workspaces::Workspace;

use super::super::{DesiredResources, InjectionMaterialization};
use super::{
    FIELD_MANAGER, KubernetesCoordinator, ReconcileError, restart_generation_is_stale,
    verify_existing,
};

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
        self.apply_namespace(namespace_name, workspace.id, desired)
            .await?;
        self.apply_access_identity(namespace_name, workspace, desired)
            .await?;
        self.apply_injections(workspace.id, &desired.injections)
            .await?;
        self.apply_workspace_identity(namespace_name, workspace.id, desired)
            .await?;
        self.apply_workload(namespace_name, workspace, desired)
            .await?;
        self.apply_network(namespace_name, workspace.id, desired)
            .await
    }

    async fn apply_namespace(
        &self,
        namespace_name: &str,
        workspace_id: Uuid,
        desired: &DesiredResources,
    ) -> Result<(), ReconcileError> {
        let namespaces = Api::<Namespace>::all(self.client.clone());
        verify_existing(&namespaces, namespace_name, &self.builder, workspace_id).await?;
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
        let service_accounts =
            Api::<ServiceAccount>::namespaced(self.client.clone(), namespace_name);
        let cluster_role_bindings = Api::<ClusterRoleBinding>::all(self.client.clone());
        let binding_name = self.builder.cluster_admin_binding_name(&workspace.short_id);
        if let Some(service_account) = &desired.service_account {
            verify_existing(
                &service_accounts,
                "workspace-admin",
                &self.builder,
                workspace_id,
            )
            .await?;
            service_accounts
                .patch("workspace-admin", &apply, &Patch::Apply(service_account))
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
            if let Some(existing) = service_accounts.get_opt("workspace-admin").await? {
                self.builder
                    .verify_delete_ownership(&existing.metadata, workspace_id)?;
                service_accounts
                    .delete("workspace-admin", &DeleteParams::default())
                    .await?;
            }
        }
        Ok(())
    }

    async fn apply_workspace_identity(
        &self,
        namespace_name: &str,
        workspace_id: Uuid,
        desired: &DesiredResources,
    ) -> Result<(), ReconcileError> {
        let apply = PatchParams::apply(FIELD_MANAGER);
        let secrets = Api::<Secret>::namespaced(self.client.clone(), namespace_name);
        verify_existing(
            &secrets,
            "workspace-ssh-identity",
            &self.builder,
            workspace_id,
        )
        .await?;
        secrets
            .patch(
                "workspace-ssh-identity",
                &apply,
                &Patch::Apply(&desired.ssh_identity),
            )
            .await?;

        let config_maps = Api::<ConfigMap>::namespaced(self.client.clone(), namespace_name);
        verify_existing(
            &config_maps,
            "workspace-config",
            &self.builder,
            workspace_id,
        )
        .await?;
        config_maps
            .patch(
                "workspace-config",
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
        let apply = PatchParams::apply(FIELD_MANAGER);
        let services = Api::<Service>::namespaced(self.client.clone(), namespace_name);
        verify_existing(&services, "workspace", &self.builder, workspace_id).await?;
        services
            .patch("workspace", &apply, &Patch::Apply(&desired.service))
            .await?;
        if let Some(service) = &desired.internal_ssh_service {
            verify_existing(&services, "workspace-ssh", &self.builder, workspace_id).await?;
            services
                .patch("workspace-ssh", &apply, &Patch::Apply(service))
                .await?;
        } else if let Some(existing) = services.get_opt("workspace-ssh").await? {
            self.builder
                .verify_delete_ownership(&existing.metadata, workspace_id)?;
            services
                .delete("workspace-ssh", &DeleteParams::default())
                .await?;
        }
        let stateful_sets = Api::<StatefulSet>::namespaced(self.client.clone(), namespace_name);
        verify_existing(&stateful_sets, "workspace", &self.builder, workspace_id).await?;
        stateful_sets
            .patch("workspace", &apply, &Patch::Apply(&desired.stateful_set))
            .await?;
        let persistent_volume_claims =
            Api::<PersistentVolumeClaim>::namespaced(self.client.clone(), namespace_name);
        if let Some(existing) = persistent_volume_claims
            .get_opt("workspace-data-workspace-0")
            .await?
        {
            self.builder
                .verify_delete_ownership(&existing.metadata, workspace_id)?;
            persistent_volume_claims
                .patch(
                    "workspace-data-workspace-0",
                    &PatchParams::default(),
                    &Patch::Merge(&serde_json::json!({
                        "metadata": {"labels": desired.stateful_set.metadata.labels}
                    })),
                )
                .await?;
        }
        if workspace.state == crate::workspaces::WorkspaceState::Restarting {
            let pods = Api::<Pod>::namespaced(self.client.clone(), namespace_name);
            if let Some(pod) = pods.get_opt("workspace-0").await? {
                self.builder
                    .verify_delete_ownership(&pod.metadata, workspace_id)?;
                if restart_generation_is_stale(&pod, workspace.generation) {
                    pods.delete("workspace-0", &DeleteParams::default()).await?;
                }
            }
        }
        Ok(())
    }

    async fn apply_network(
        &self,
        namespace_name: &str,
        workspace_id: Uuid,
        desired: &DesiredResources,
    ) -> Result<(), ReconcileError> {
        let apply = PatchParams::apply(FIELD_MANAGER);
        let network_policies =
            Api::<NetworkPolicy>::namespaced(self.client.clone(), namespace_name);
        verify_existing(
            &network_policies,
            "workspace-ingress",
            &self.builder,
            workspace_id,
        )
        .await?;
        network_policies
            .patch(
                "workspace-ingress",
                &apply,
                &Patch::Apply(&desired.network_policy),
            )
            .await?;
        let ingresses = Api::<Ingress>::namespaced(self.client.clone(), namespace_name);
        if let Some(existing) = ingresses.get_opt("web-shell").await? {
            self.builder
                .verify_delete_ownership(&existing.metadata, workspace_id)?;
            if desired.web_shell_ingress.is_none() {
                ingresses
                    .delete("web-shell", &DeleteParams::default())
                    .await?;
            }
        }
        if let Some(ingress) = &desired.web_shell_ingress {
            ingresses
                .patch("web-shell", &apply, &Patch::Apply(ingress))
                .await?;
        }
        Ok(())
    }

    async fn apply_injections(
        &self,
        workspace_id: Uuid,
        materialization: &InjectionMaterialization,
    ) -> Result<(), ReconcileError> {
        let namespace = materialization
            .file_config_map
            .metadata
            .namespace
            .as_deref()
            .ok_or(ReconcileError::MissingObjectName)?;
        let apply = PatchParams::apply(FIELD_MANAGER);
        let secrets = Api::<Secret>::namespaced(self.client.clone(), namespace);
        verify_existing(
            &secrets,
            "workspace-environment-secret",
            &self.builder,
            workspace_id,
        )
        .await?;
        secrets
            .patch(
                "workspace-environment-secret",
                &apply,
                &Patch::Apply(&materialization.environment_secret),
            )
            .await?;
        verify_existing(
            &secrets,
            "workspace-files-secret",
            &self.builder,
            workspace_id,
        )
        .await?;
        secrets
            .patch(
                "workspace-files-secret",
                &apply,
                &Patch::Apply(&materialization.file_secret),
            )
            .await?;
        let config_maps = Api::<ConfigMap>::namespaced(self.client.clone(), namespace);
        verify_existing(
            &config_maps,
            "workspace-environment-config",
            &self.builder,
            workspace_id,
        )
        .await?;
        config_maps
            .patch(
                "workspace-environment-config",
                &apply,
                &Patch::Apply(&materialization.environment_config_map),
            )
            .await?;
        verify_existing(
            &config_maps,
            "workspace-files-config",
            &self.builder,
            workspace_id,
        )
        .await?;
        config_maps
            .patch(
                "workspace-files-config",
                &apply,
                &Patch::Apply(&materialization.file_config_map),
            )
            .await?;
        Ok(())
    }
}
