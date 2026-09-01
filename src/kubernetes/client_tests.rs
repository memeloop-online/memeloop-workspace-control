mod node_port_tests {
    use k8s_openapi::api::core::v1::{Service, ServicePort, ServiceSpec};

    use super::super::client::node_port_from_service;

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

mod coordinator_tests {
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

    use super::super::client::{
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
                port_mapping_domain: None,
                control_plane_internal_service_dns: "mwc-internal.control.svc.cluster.local"
                    .to_owned(),
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
