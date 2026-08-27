use std::collections::BTreeMap;

use k8s_openapi::{
    api::{
        apps::v1::{StatefulSet, StatefulSetSpec},
        core::v1::{
            ConfigMap, ConfigMapVolumeSource, Container, ContainerPort, EmptyDirVolumeSource,
            Namespace, PersistentVolumeClaim, PersistentVolumeClaimSpec, PodSpec, PodTemplateSpec,
            ResourceRequirements, SecretVolumeSource, Service, Volume, VolumeResourceRequirements,
        },
        networking::v1::{Ingress, NetworkPolicy},
    },
    apimachinery::pkg::{
        api::resource::Quantity,
        apis::meta::v1::{LabelSelector, ObjectMeta},
    },
};
use thiserror::Error;
use uuid::Uuid;

use crate::{config::InstallationId, quota::Resources, workspaces::WorkspaceState};

mod client;
mod higress;
mod materialization;
mod network_policy;
mod ownership;
mod resource_helpers;
mod runtime_profiles;

pub use client::{DeleteProgress, KubernetesCoordinator, ReconcileError, workspace_ssh_node_port};
pub use materialization::{InjectionMaterialization, MaterializationError};
pub use ownership::OwnershipError;

pub(crate) use resource_helpers::namespaced_metadata;
use resource_helpers::{internal_ssh_service, mount, pod_labels, service, workspace_config};
use runtime_profiles::RuntimeProfile;

pub const OWNER_INSTALLATION_LABEL: &str = "workspace.memeloop.dev/owner-installation";
pub const WORKSPACE_ID_LABEL: &str = "workspace.memeloop.dev/workspace-id";
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
    pub higress_gateway_name: String,
    pub higress_https_section_name: String,
    pub internal_ssh_node_port_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct WorkspaceResourceSpec {
    pub id: Uuid,
    pub short_id: String,
    pub image: String,
    pub resources: Resources,
    pub access_mode: crate::workspaces::AccessMode,
    pub state: WorkspaceState,
    pub generation: u64,
    pub runtime_profile: crate::workspaces::WorkspaceRuntimeProfile,
}

#[derive(Debug)]
pub struct DesiredResources {
    pub namespace: Namespace,
    pub service: Service,
    pub internal_ssh_service: Option<Service>,
    pub stateful_set: StatefulSet,
    pub network_policy: NetworkPolicy,
    pub injections: InjectionMaterialization,
    pub workspace_config: ConfigMap,
    pub ssh_identity: k8s_openapi::api::core::v1::Secret,
    pub web_shell_ingress: Option<Ingress>,
}

impl ResourceBuilder {
    pub fn build(&self, workspace: &WorkspaceResourceSpec) -> Result<DesiredResources, BuildError> {
        if matches!(
            workspace.state,
            WorkspaceState::Deleting | WorkspaceState::Deleted
        ) {
            return Err(BuildError::WorkspaceBeingDeleted);
        }
        if workspace.image.trim().is_empty() {
            return Err(BuildError::EmptyImage);
        }

        let namespace_name = self
            .installation_id
            .workspace_namespace(&workspace.short_id)?;
        let labels = self.labels(workspace.id);
        let pod_labels = pod_labels(&labels);
        let replicas = match workspace.state {
            WorkspaceState::Stopping | WorkspaceState::Stopped | WorkspaceState::Failed => 0,
            WorkspaceState::Provisioning
            | WorkspaceState::Ready
            | WorkspaceState::Starting
            | WorkspaceState::Restarting => 1,
            WorkspaceState::Deleting | WorkspaceState::Deleted => unreachable!(),
        };

        let injections = materialization::build(&namespace_name, &labels, &[])?;
        Ok(DesiredResources {
            namespace: Namespace {
                metadata: ObjectMeta {
                    name: Some(namespace_name.clone()),
                    labels: Some(labels.clone()),
                    ..ObjectMeta::default()
                },
                ..Namespace::default()
            },
            service: service(&namespace_name, &labels, &pod_labels),
            internal_ssh_service: internal_ssh_service(
                &namespace_name,
                &labels,
                &pod_labels,
                workspace.access_mode,
                self.internal_ssh_node_port_enabled,
            ),
            stateful_set: self.stateful_set(
                &namespace_name,
                &labels,
                &pod_labels,
                workspace,
                replicas,
            ),
            network_policy: network_policy::build(
                &namespace_name,
                &labels,
                &pod_labels,
                &self.higress_namespace,
                &self.higress_pod_labels,
                &self.higress_source_cidrs,
                &self.jump_host_namespace,
                &self.jump_host_pod_labels,
                workspace.access_mode,
                self.internal_ssh_node_port_enabled,
            ),
            injections,
            workspace_config: workspace_config(
                &namespace_name,
                &labels,
                RuntimeProfile::for_workspace(workspace.runtime_profile),
            ),
            ssh_identity: resource_helpers::ssh_identity(&namespace_name, &labels, None),
            web_shell_ingress: self.web_shell_domain.as_ref().map(|domain| {
                higress::web_shell_ingress(&namespace_name, &labels, &workspace.short_id, domain)
            }),
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

    pub fn materialize_injections(
        &self,
        workspace_id: Uuid,
        workspace_short_id: &str,
        resolved: &[crate::injections::ResolvedInjection],
    ) -> Result<InjectionMaterialization, MaterializationError> {
        let namespace = self
            .installation_id
            .workspace_namespace(workspace_short_id)
            .map_err(MaterializationError::Config)?;
        materialization::build(&namespace, &self.labels(workspace_id), resolved)
    }

    pub fn materialize_ssh_identity(
        &self,
        workspace_id: Uuid,
        workspace_short_id: &str,
        identity: &crate::storage::WorkspaceSshIdentity,
    ) -> Result<k8s_openapi::api::core::v1::Secret, crate::config::ConfigError> {
        let namespace = self
            .installation_id
            .workspace_namespace(workspace_short_id)?;
        Ok(resource_helpers::ssh_identity(
            &namespace,
            &self.labels(workspace_id),
            Some(identity),
        ))
    }

    fn labels(&self, workspace_id: Uuid) -> BTreeMap<String, String> {
        BTreeMap::from([
            (
                OWNER_INSTALLATION_LABEL.to_owned(),
                self.installation_id.to_string(),
            ),
            (WORKSPACE_ID_LABEL.to_owned(), workspace_id.to_string()),
            (
                MANAGED_BY_LABEL.to_owned(),
                "memeloop-workspace-control".to_owned(),
            ),
        ])
    }

    fn stateful_set(
        &self,
        namespace: &str,
        labels: &BTreeMap<String, String>,
        pod_labels: &BTreeMap<String, String>,
        workspace: &WorkspaceResourceSpec,
        replicas: i32,
    ) -> StatefulSet {
        let profile = RuntimeProfile::for_workspace(workspace.runtime_profile);
        let mut requests = profile.resource_requests(workspace.resources);
        let mut limits = profile.resource_limits(workspace.resources);
        if workspace.resources.gpu_count > 0 {
            let quantity = Quantity(workspace.resources.gpu_count.to_string());
            requests.insert("nvidia.com/gpu".to_owned(), quantity.clone());
            limits.insert("nvidia.com/gpu".to_owned(), quantity);
        }
        let workspace_resources = ResourceRequirements {
            requests: Some(requests),
            limits: Some(limits),
            ..ResourceRequirements::default()
        };
        let mut init_containers = vec![profile.workspace_init_container(&workspace.image)];
        if let Some(buildkit_init) = profile.buildkit_init_container() {
            init_containers.push(buildkit_init);
        }
        let mut containers =
            vec![profile.workspace_container(&workspace.image, workspace_resources)];
        if let Some(buildkit) = profile.buildkit_container() {
            containers.push(buildkit);
        }
        containers.push(Container {
            name: "ttyd".to_owned(),
            image: Some(self.ttyd_image.clone()),
            command: Some(vec!["ttyd".to_owned()]),
            args: Some(vec![
                "--port".to_owned(),
                "7681".to_owned(),
                "--writable".to_owned(),
                "--base-path".to_owned(),
                format!("/shell/{}", workspace.short_id),
                "ssh".to_owned(),
                "-p".to_owned(),
                "2222".to_owned(),
                "-o".to_owned(),
                "StrictHostKeyChecking=yes".to_owned(),
                "-o".to_owned(),
                "UserKnownHostsFile=/etc/ssh/platform/known_hosts".to_owned(),
                "-o".to_owned(),
                "BatchMode=yes".to_owned(),
                "-i".to_owned(),
                "/etc/ssh/platform/ttyd_client_key".to_owned(),
                format!("{}@127.0.0.1", profile.login_user),
            ]),
            ports: Some(vec![ContainerPort {
                container_port: 7681,
                name: Some("web-shell".to_owned()),
                protocol: Some("TCP".to_owned()),
                ..ContainerPort::default()
            }]),
            volume_mounts: Some(vec![mount("runtime-ssh", "/etc/ssh/platform", true)]),
            ..Container::default()
        });
        let pod_spec = PodSpec {
            automount_service_account_token: Some(false),
            init_containers: Some(init_containers),
            containers,
            affinity: profile.affinity(),
            node_selector: profile.node_selector(),
            security_context: profile.pod_security_context(),
            // Harbor's current library images are pullable without credentials. Keep this
            // explicit so a future private-image resolver can add a namespaced pull Secret
            // without changing any runtime-profile semantics.
            image_pull_secrets: profile.image_pull_secrets(),
            volumes: Some(vec![
                Volume {
                    name: "ssh-identity".to_owned(),
                    secret: Some(SecretVolumeSource {
                        secret_name: Some("workspace-ssh-identity".to_owned()),
                        default_mode: Some(0o400),
                        ..SecretVolumeSource::default()
                    }),
                    ..Volume::default()
                },
                Volume {
                    name: "workspace-files-secret".to_owned(),
                    secret: Some(SecretVolumeSource {
                        secret_name: Some("workspace-files-secret".to_owned()),
                        ..SecretVolumeSource::default()
                    }),
                    ..Volume::default()
                },
                Volume {
                    name: "workspace-files-config".to_owned(),
                    config_map: Some(ConfigMapVolumeSource {
                        name: "workspace-files-config".to_owned(),
                        ..ConfigMapVolumeSource::default()
                    }),
                    ..Volume::default()
                },
                Volume {
                    name: "workspace-config".to_owned(),
                    config_map: Some(ConfigMapVolumeSource {
                        name: "workspace-config".to_owned(),
                        default_mode: Some(0o555),
                        ..ConfigMapVolumeSource::default()
                    }),
                    ..Volume::default()
                },
                Volume {
                    name: "runtime-ssh".to_owned(),
                    empty_dir: Some(EmptyDirVolumeSource::default()),
                    ..Volume::default()
                },
            ]),
            ..PodSpec::default()
        };
        StatefulSet {
            metadata: namespaced_metadata("workspace", namespace, labels),
            spec: Some(StatefulSetSpec {
                replicas: Some(replicas),
                service_name: Some("workspace".to_owned()),
                selector: LabelSelector {
                    match_labels: Some(pod_labels.clone()),
                    ..LabelSelector::default()
                },
                template: PodTemplateSpec {
                    metadata: Some(ObjectMeta {
                        labels: Some(pod_labels.clone()),
                        annotations: Some(BTreeMap::from([(
                            "workspace.memeloop.dev/generation".to_owned(),
                            workspace.generation.to_string(),
                        )])),
                        ..ObjectMeta::default()
                    }),
                    spec: Some(pod_spec),
                },
                volume_claim_templates: Some(vec![PersistentVolumeClaim {
                    metadata: ObjectMeta {
                        name: Some("workspace-data".to_owned()),
                        labels: Some(labels.clone()),
                        ..ObjectMeta::default()
                    },
                    spec: Some(PersistentVolumeClaimSpec {
                        access_modes: Some(vec!["ReadWriteOnce".to_owned()]),
                        storage_class_name: self.storage_class_name.clone(),
                        resources: Some(VolumeResourceRequirements {
                            requests: Some(BTreeMap::from([(
                                "storage".to_owned(),
                                Quantity(format!("{}Gi", workspace.resources.disk_gib)),
                            )])),
                            ..VolumeResourceRequirements::default()
                        }),
                        ..PersistentVolumeClaimSpec::default()
                    }),
                    ..PersistentVolumeClaim::default()
                }]),
                ..StatefulSetSpec::default()
            }),
            ..StatefulSet::default()
        }
    }
}

#[derive(Debug, Error)]
pub enum BuildError {
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),
    #[error(transparent)]
    Materialization(#[from] MaterializationError),
    #[error("cannot build desired runtime resources while workspace is being deleted")]
    WorkspaceBeingDeleted,
    #[error("workspace image must not be empty")]
    EmptyImage,
}
