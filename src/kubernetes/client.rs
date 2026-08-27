use std::fmt::Debug;

use k8s_openapi::api::{
    apps::v1::StatefulSet,
    core::v1::{ConfigMap, Namespace, Secret, Service},
    networking::v1::NetworkPolicy,
};
use kube::{
    Api, Client,
    api::{DeleteParams, Patch, PatchParams},
    core::DynamicObject,
};
use serde::de::DeserializeOwned;
use thiserror::Error;
use uuid::Uuid;

use super::{
    BuildError, InjectionMaterialization, OwnershipError, ResourceBuilder, WorkspaceResourceSpec,
};

const FIELD_MANAGER: &str = "memeloop-workspace-control";

#[derive(Clone)]
pub struct KubernetesCoordinator {
    client: Client,
    builder: ResourceBuilder,
}

impl KubernetesCoordinator {
    pub fn new(client: Client, builder: ResourceBuilder) -> Self {
        Self { client, builder }
    }

    pub async fn reconcile(&self, workspace: &WorkspaceResourceSpec) -> Result<(), ReconcileError> {
        let desired = self.builder.build(workspace)?;
        self.apply_desired(workspace.id, &desired).await
    }

    pub async fn reconcile_with_injections(
        &self,
        workspace: &WorkspaceResourceSpec,
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
        self.apply_desired(workspace.id, &desired).await
    }

    async fn apply_desired(
        &self,
        workspace_id: Uuid,
        desired: &super::DesiredResources,
    ) -> Result<(), ReconcileError> {
        let namespace_name = desired
            .namespace
            .metadata
            .name
            .as_deref()
            .ok_or(ReconcileError::MissingObjectName)?;
        let apply = PatchParams::apply(FIELD_MANAGER);

        let namespaces = Api::<Namespace>::all(self.client.clone());
        verify_existing(&namespaces, namespace_name, &self.builder, workspace_id).await?;
        namespaces
            .patch(namespace_name, &apply, &Patch::Apply(&desired.namespace))
            .await?;
        self.apply_injections(workspace_id, &desired.injections)
            .await?;

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

        let services = Api::<Service>::namespaced(self.client.clone(), namespace_name);
        verify_existing(&services, "workspace", &self.builder, workspace_id).await?;
        services
            .patch("workspace", &apply, &Patch::Apply(&desired.service))
            .await?;
        let stateful_sets = Api::<StatefulSet>::namespaced(self.client.clone(), namespace_name);
        verify_existing(&stateful_sets, "workspace", &self.builder, workspace_id).await?;
        stateful_sets
            .patch("workspace", &apply, &Patch::Apply(&desired.stateful_set))
            .await?;
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
        // Internal installations deliberately do not require Gateway API CRDs. A request to a
        // missing API group returns a plain-text 404 that kube cannot normalize as NotFound, so
        // do not construct or query the dynamic API unless Web Shell routing is configured.
        if self.builder.web_shell_domain.is_some() {
            let routes = Api::<DynamicObject>::namespaced_with(
                self.client.clone(),
                namespace_name,
                &super::higress::http_route_resource(),
            );
            if let Some(existing) = routes.get_opt("web-shell").await? {
                self.builder
                    .verify_delete_ownership(&existing.metadata, workspace_id)?;
                if desired.web_shell_route.is_none() {
                    routes.delete("web-shell", &DeleteParams::default()).await?;
                }
            }
            if let Some(route) = &desired.web_shell_route {
                routes
                    .patch("web-shell", &apply, &Patch::Apply(route))
                    .await?;
            }
        }
        Ok(())
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
        let namespaces = Api::<Namespace>::all(self.client.clone());
        let Some(namespace) = namespaces.get_opt(&namespace_name).await? else {
            return Ok(DeleteProgress::Gone);
        };

        self.builder
            .verify_delete_ownership(&namespace.metadata, workspace_id)?;
        if self.builder.web_shell_domain.is_some() {
            let routes = Api::<DynamicObject>::namespaced_with(
                self.client.clone(),
                &namespace_name,
                &super::higress::http_route_resource(),
            );
            if let Some(route) = routes.get_opt("web-shell").await? {
                self.builder
                    .verify_delete_ownership(&route.metadata, workspace_id)?;
                routes.delete("web-shell", &DeleteParams::default()).await?;
                // Keep deletion deliberately staged: do not start Namespace removal
                // until the externally visible Web Shell route is confirmed gone.
                return Ok(DeleteProgress::DeletionRequested);
            }
        }
        if namespace.metadata.deletion_timestamp.is_some() {
            return Ok(DeleteProgress::Terminating);
        }
        namespaces
            .delete(&namespace_name, &DeleteParams::default())
            .await?;
        Ok(DeleteProgress::DeletionRequested)
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
