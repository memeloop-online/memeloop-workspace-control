use std::fmt::Debug;

use k8s_openapi::api::{
    apps::v1::StatefulSet,
    core::v1::{ConfigMap, Namespace, Pod, Secret, Service, ServiceAccount},
    networking::v1::{Ingress, NetworkPolicy},
    rbac::v1::ClusterRoleBinding,
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
    use std::{
        collections::BTreeMap,
        convert::Infallible,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
    };

    use axum::{
        body::Body,
        http::{Method, Request, Response, StatusCode},
    };
    use k8s_openapi::{api::core::v1::Pod, apimachinery::pkg::apis::meta::v1::ObjectMeta};
    use tower::service_fn;
    use uuid::Uuid;

    use super::{
        DeleteProgress, KubernetesCoordinator, ReconcileError, restart_generation_is_stale,
    };
    use crate::kubernetes::ResourceBuilder;

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

    #[tokio::test]
    async fn deletion_removes_owned_cluster_binding_then_service_account_then_namespace() {
        let workspace_id = Uuid::now_v7();
        let mock = Arc::new(DeleteMock::new("public-a", workspace_id));
        let coordinator = coordinator(mock.clone());

        assert_eq!(
            coordinator
                .delete_or_confirm(workspace_id, "01jabc")
                .await
                .unwrap(),
            DeleteProgress::DeletionRequested
        );
        assert!(!mock.binding_exists.load(Ordering::SeqCst));
        assert!(mock.service_account_exists.load(Ordering::SeqCst));

        assert_eq!(
            coordinator
                .delete_or_confirm(workspace_id, "01jabc")
                .await
                .unwrap(),
            DeleteProgress::DeletionRequested
        );
        assert!(!mock.service_account_exists.load(Ordering::SeqCst));
        assert!(mock.namespace_exists.load(Ordering::SeqCst));

        assert_eq!(
            coordinator
                .delete_or_confirm(workspace_id, "01jabc")
                .await
                .unwrap(),
            DeleteProgress::DeletionRequested
        );
        assert!(!mock.namespace_exists.load(Ordering::SeqCst));
        assert_eq!(
            coordinator
                .delete_or_confirm(workspace_id, "01jabc")
                .await
                .unwrap(),
            DeleteProgress::Gone
        );

        let requests = mock.requests.lock().unwrap();
        let deletes = requests
            .iter()
            .filter(|(method, _)| method == Method::DELETE.as_str())
            .map(|(_, path)| path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            deletes,
            [
                "/apis/rbac.authorization.k8s.io/v1/clusterrolebindings/mwc-public-a-01jabc-admin",
                "/api/v1/namespaces/ws-public-a-01jabc/serviceaccounts/workspace-admin",
                "/api/v1/namespaces/ws-public-a-01jabc",
            ]
        );
    }

    #[tokio::test]
    async fn deletion_never_removes_another_installations_cluster_binding() {
        let workspace_id = Uuid::now_v7();
        let mock = Arc::new(DeleteMock::new("other", workspace_id));
        let coordinator = coordinator(mock.clone());
        assert!(matches!(
            coordinator.delete_or_confirm(workspace_id, "01jabc").await,
            Err(ReconcileError::Ownership(_))
        ));
        assert!(mock.binding_exists.load(Ordering::SeqCst));
        assert!(
            mock.requests
                .lock()
                .unwrap()
                .iter()
                .all(|(method, _)| method != Method::DELETE.as_str())
        );
    }

    struct DeleteMock {
        binding_owner: String,
        workspace_id: Uuid,
        binding_exists: AtomicBool,
        service_account_exists: AtomicBool,
        namespace_exists: AtomicBool,
        requests: Mutex<Vec<(String, String)>>,
    }

    impl DeleteMock {
        fn new(binding_owner: &str, workspace_id: Uuid) -> Self {
            Self {
                binding_owner: binding_owner.to_owned(),
                workspace_id,
                binding_exists: AtomicBool::new(true),
                service_account_exists: AtomicBool::new(true),
                namespace_exists: AtomicBool::new(true),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn response(&self, method: &Method, path: &str) -> Response<Body> {
            self.requests
                .lock()
                .unwrap()
                .push((method.to_string(), path.to_owned()));
            let binding_path =
                "/apis/rbac.authorization.k8s.io/v1/clusterrolebindings/mwc-public-a-01jabc-admin";
            let namespace_path = "/api/v1/namespaces/ws-public-a-01jabc";
            let service_account_path =
                "/api/v1/namespaces/ws-public-a-01jabc/serviceaccounts/workspace-admin";
            let ingress_path =
                "/apis/networking.k8s.io/v1/namespaces/ws-public-a-01jabc/ingresses/web-shell";
            match (method, path) {
                (&Method::GET, value) if value == binding_path => {
                    if self.binding_exists.load(Ordering::SeqCst) {
                        json_response(
                            StatusCode::OK,
                            serde_json::json!({
                                "apiVersion": "rbac.authorization.k8s.io/v1",
                                "kind": "ClusterRoleBinding",
                                "metadata": {
                                    "name": "mwc-public-a-01jabc-admin",
                                    "labels": ownership_labels(&self.binding_owner, self.workspace_id),
                                },
                                "roleRef": {
                                    "apiGroup": "rbac.authorization.k8s.io",
                                    "kind": "ClusterRole",
                                    "name": "cluster-admin",
                                },
                            }),
                        )
                    } else {
                        not_found()
                    }
                }
                (&Method::DELETE, value) if value == binding_path => {
                    self.binding_exists.store(false, Ordering::SeqCst);
                    success()
                }
                (&Method::GET, value) if value == namespace_path => {
                    if self.namespace_exists.load(Ordering::SeqCst) {
                        json_response(
                            StatusCode::OK,
                            serde_json::json!({
                                "apiVersion": "v1",
                                "kind": "Namespace",
                                "metadata": {
                                    "name": "ws-public-a-01jabc",
                                    "labels": ownership_labels("public-a", self.workspace_id),
                                },
                            }),
                        )
                    } else {
                        not_found()
                    }
                }
                (&Method::DELETE, value) if value == namespace_path => {
                    self.namespace_exists.store(false, Ordering::SeqCst);
                    success()
                }
                (&Method::GET, value) if value == service_account_path => {
                    if self.service_account_exists.load(Ordering::SeqCst) {
                        json_response(
                            StatusCode::OK,
                            serde_json::json!({
                                "apiVersion": "v1",
                                "kind": "ServiceAccount",
                                "metadata": {
                                    "name": "workspace-admin",
                                    "namespace": "ws-public-a-01jabc",
                                    "labels": ownership_labels("public-a", self.workspace_id),
                                },
                            }),
                        )
                    } else {
                        not_found()
                    }
                }
                (&Method::DELETE, value) if value == service_account_path => {
                    self.service_account_exists.store(false, Ordering::SeqCst);
                    success()
                }
                (&Method::GET, value) if value == ingress_path => not_found(),
                _ => panic!("unexpected Kubernetes request: {method} {path}"),
            }
        }
    }

    fn coordinator(mock: Arc<DeleteMock>) -> KubernetesCoordinator {
        let service = service_fn(move |request: Request<kube::client::Body>| {
            let mock = mock.clone();
            async move { Ok::<_, Infallible>(mock.response(request.method(), request.uri().path())) }
        });
        KubernetesCoordinator::new(
            kube::Client::new(service, "default"),
            ResourceBuilder {
                installation_id: "public-a".parse().unwrap(),
                ttyd_image: "example/ttyd:1".to_owned(),
                higress_namespace: "higress-system".to_owned(),
                higress_pod_labels: BTreeMap::new(),
                higress_source_cidrs: Vec::new(),
                jump_host_namespace: "access".to_owned(),
                jump_host_pod_labels: BTreeMap::new(),
                storage_class_name: None,
                web_shell_domain: None,
                higress_gateway_name: "higress".to_owned(),
                higress_https_section_name: "https".to_owned(),
                internal_ssh_node_port_enabled: false,
            },
        )
    }

    fn ownership_labels(owner: &str, workspace_id: Uuid) -> serde_json::Value {
        serde_json::json!({
            "workspace.memeloop.dev/owner-installation": owner,
            "workspace.memeloop.dev/workspace-id": workspace_id.to_string(),
        })
    }

    fn json_response(status: StatusCode, value: serde_json::Value) -> Response<Body> {
        Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .body(Body::from(value.to_string()))
            .unwrap()
    }

    fn not_found() -> Response<Body> {
        json_response(
            StatusCode::NOT_FOUND,
            serde_json::json!({
                "apiVersion": "v1",
                "kind": "Status",
                "status": "Failure",
                "reason": "NotFound",
                "message": "not found",
                "code": 404,
            }),
        )
    }

    fn success() -> Response<Body> {
        json_response(
            StatusCode::OK,
            serde_json::json!({
                "apiVersion": "v1",
                "kind": "Status",
                "status": "Success",
                "code": 200,
            }),
        )
    }
}
