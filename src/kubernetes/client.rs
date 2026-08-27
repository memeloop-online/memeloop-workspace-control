use std::fmt::Debug;

use k8s_openapi::api::{
    apps::v1::StatefulSet,
    core::v1::{ConfigMap, Namespace, Pod, Secret, Service},
    networking::v1::{Ingress, NetworkPolicy},
};
use kube::{
    Api, Client,
    api::{DeleteParams, Patch, PatchParams},
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
        self.apply_desired(workspace, &desired).await
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
        self.apply_desired(workspace, &desired).await
    }

    async fn apply_desired(
        &self,
        workspace: &WorkspaceResourceSpec,
        desired: &super::DesiredResources,
    ) -> Result<(), ReconcileError> {
        let workspace_id = workspace.id;
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

fn node_port_from_service(service: &Service) -> Option<u16> {
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

#[cfg(test)]
mod node_port_tests {
    use k8s_openapi::api::core::v1::{ServicePort, ServiceSpec};

    use super::*;

    #[test]
    fn reads_only_the_assigned_ssh_node_port() {
        let service = Service {
            spec: Some(ServiceSpec {
                ports: Some(vec![
                    ServicePort {
                        name: Some("web-shell".to_owned()),
                        node_port: Some(32767),
                        ..ServicePort::default()
                    },
                    ServicePort {
                        name: Some("ssh".to_owned()),
                        node_port: Some(31022),
                        ..ServicePort::default()
                    },
                ]),
                ..ServiceSpec::default()
            }),
            ..Service::default()
        };
        assert_eq!(node_port_from_service(&service), Some(31022));
    }
}

fn restart_generation_is_stale(pod: &Pod, expected: u64) -> bool {
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use k8s_openapi::{api::core::v1::Pod, apimachinery::pkg::apis::meta::v1::ObjectMeta};

    use super::restart_generation_is_stale;

    #[test]
    fn restart_deletes_only_a_pod_from_an_older_generation() {
        let pod = |generation: Option<&str>| Pod {
            metadata: ObjectMeta {
                annotations: generation.map(|value| {
                    BTreeMap::from([(
                        "workspace.memeloop.dev/generation".to_owned(),
                        value.to_owned(),
                    )])
                }),
                ..ObjectMeta::default()
            },
            ..Pod::default()
        };

        assert!(restart_generation_is_stale(&pod(None), 2));
        assert!(restart_generation_is_stale(&pod(Some("1")), 2));
        assert!(!restart_generation_is_stale(&pod(Some("2")), 2));
    }
}
