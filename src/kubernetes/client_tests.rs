#[path = "client_tests/ttyd_mtls.rs"]
mod ttyd_mtls;

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
        http::{Method, Request, Response, StatusCode, Uri},
    };
    use k8s_openapi::{api::core::v1::Pod, apimachinery::pkg::apis::meta::v1::ObjectMeta};
    use tower::service_fn;
    use uuid::Uuid;

    use super::super::client::{
        DeleteProgress, KubernetesCoordinator, ReconcileError, restart_generation_is_stale,
    };
    use crate::{
        kubernetes::{InternetEgressConfig, ResourceBuilder},
        quota::Resources,
        templates::WorkspaceTemplateSpec,
        workspace_runtime::WorkspaceRuntimeIdentity,
        workspaces::{AccessMode, Workspace, WorkspaceHomeVolumeBinding, WorkspaceState},
    };

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
    async fn deletion_waits_for_pods_then_deletes_only_the_owned_pvc() {
        let workspace_id = workspace_id();
        let workspace = workspace(workspace_id);
        let mock = Arc::new(WorkspaceDeleteMock::new("public-a", workspace_id));
        let coordinator = workspace_coordinator(mock.clone());

        assert_eq!(
            coordinator.delete_or_confirm(&workspace).await.unwrap(),
            DeleteProgress::Terminating
        );
        assert!(mock.pvc_exists.load(Ordering::SeqCst));
        assert!(
            mock.requests
                .lock()
                .unwrap()
                .iter()
                .all(|(method, _)| method != Method::DELETE.as_str())
        );

        mock.pod_exists.store(false, Ordering::SeqCst);
        assert_eq!(
            coordinator.delete_or_confirm(&workspace).await.unwrap(),
            DeleteProgress::DeletionRequested
        );
        assert!(!mock.pvc_exists.load(Ordering::SeqCst));
        assert_eq!(
            coordinator.delete_or_confirm(&workspace).await.unwrap(),
            DeleteProgress::Gone
        );
        let requests = mock.requests.lock().unwrap();
        let deletes = requests
            .iter()
            .filter(|(method, _)| method == Method::DELETE.as_str())
            .map(|(_, uri)| uri.trim_end_matches('?'))
            .collect::<Vec<_>>();
        assert_eq!(
            deletes,
            [
                "/api/v1/namespaces/memeloop-workspace-control/persistentvolumeclaims/workspace-data-w-8000000000000001-0"
            ]
        );
    }

    #[tokio::test]
    async fn deletion_rejects_a_pod_with_wrong_ownership_without_deleting_anything() {
        let workspace_id = workspace_id();
        let workspace = workspace(workspace_id);
        let mock = Arc::new(WorkspaceDeleteMock::new("other", workspace_id));
        let coordinator = workspace_coordinator(mock.clone());
        assert!(matches!(
            coordinator.delete_or_confirm(&workspace).await,
            Err(ReconcileError::Ownership(_))
        ));
        assert!(
            mock.requests
                .lock()
                .unwrap()
                .iter()
                .all(|(method, _)| method != Method::DELETE.as_str())
        );
    }

    #[tokio::test]
    async fn deletion_rejects_a_target_pod_without_ownership_labels() {
        let workspace_id = workspace_id();
        let workspace = workspace(workspace_id);
        let mock = Arc::new(WorkspaceDeleteMock::without_pod_labels(workspace_id));
        let coordinator = workspace_coordinator(mock.clone());

        assert!(matches!(
            coordinator.delete_or_confirm(&workspace).await,
            Err(ReconcileError::Ownership(_))
        ));
        assert_no_pvc_delete(&mock);
    }

    #[tokio::test]
    async fn deletion_waits_for_any_pod_referencing_the_data_pvc() {
        let workspace_id = workspace_id();
        let workspace = workspace(workspace_id);
        let mock = Arc::new(WorkspaceDeleteMock::new("public-a", workspace_id));
        mock.pod_exists.store(false, Ordering::SeqCst);
        mock.other_pod_references_pvc.store(true, Ordering::SeqCst);
        let coordinator = workspace_coordinator(mock.clone());

        assert_eq!(
            coordinator.delete_or_confirm(&workspace).await.unwrap(),
            DeleteProgress::Terminating
        );
        assert!(mock.pvc_exists.load(Ordering::SeqCst));
        assert_no_pvc_delete(&mock);
    }

    #[tokio::test]
    async fn deletion_rechecks_an_unlabeled_target_pod_that_appears_in_the_pod_list() {
        let workspace_id = workspace_id();
        let workspace = workspace(workspace_id);
        let mock = Arc::new(WorkspaceDeleteMock::without_pod_labels(workspace_id));
        mock.pod_exists.store(false, Ordering::SeqCst);
        mock.target_pod_appears_in_list
            .store(true, Ordering::SeqCst);
        let coordinator = workspace_coordinator(mock.clone());

        assert!(matches!(
            coordinator.delete_or_confirm(&workspace).await,
            Err(ReconcileError::Ownership(_))
        ));
        assert_pod_list_has_no_label_selector(&mock);
        assert_no_pvc_delete(&mock);
    }

    fn workspace(id: Uuid) -> Workspace {
        Workspace {
            id,
            short_id: "8000000000000001".to_owned(),
            organization_id: Uuid::now_v7(),
            owner_id: Uuid::now_v7(),
            name: "test".to_owned(),
            template_id: None,
            home_volume_binding: None,
            node_pool: "default".to_owned(),
            runtime: WorkspaceRuntimeIdentity,
            template: WorkspaceTemplateSpec::standard(
                "example/workspace:1",
                AccessMode::Public,
                Resources {
                    cpu_millis: 1_000,
                    memory_mib: 1_024,
                    gpu_count: 0,
                    disk_gib: 10,
                },
            ),
            state: WorkspaceState::Deleting,
            generation: 1,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn workspace_id() -> Uuid {
        Uuid::parse_str("018f0000-0000-7000-8000-000000000001").unwrap()
    }

    fn bind_home(workspace: &mut Workspace, claim_name: &str) {
        workspace.home_volume_binding = Some(WorkspaceHomeVolumeBinding {
            namespace: workspace.runtime.namespace().to_owned(),
            claim_name: claim_name.to_owned(),
            claim_uid: "home-uid".to_owned(),
            capacity_gib: workspace.template.resources.disk_gib,
        });
    }

    #[tokio::test]
    async fn bound_home_is_retained_even_when_its_name_matches_the_legacy_generated_claim() {
        let mut workspace = workspace(workspace_id());
        bind_home(&mut workspace, "workspace-data-w-8000000000000001-0");
        let mock = Arc::new(WorkspaceDeleteMock::new("public-a", workspace.id));
        let coordinator = workspace_coordinator(mock.clone());
        assert_eq!(
            coordinator.delete_or_confirm(&workspace).await.unwrap(),
            DeleteProgress::Terminating
        );
        mock.pod_exists.store(false, Ordering::SeqCst);
        mock.config_exists.store(true, Ordering::SeqCst);
        assert_eq!(
            coordinator.delete_or_confirm(&workspace).await.unwrap(),
            DeleteProgress::DeletionRequested
        );
        assert!(!mock.config_exists.load(Ordering::SeqCst));
        assert_eq!(
            coordinator.delete_or_confirm(&workspace).await.unwrap(),
            DeleteProgress::Gone
        );
        assert!(mock.pvc_exists.load(Ordering::SeqCst));
        assert_no_pvc_delete(&mock);
        assert!(
            mock.requests
                .lock()
                .unwrap()
                .iter()
                .all(|(_, uri)| !uri.contains("persistentvolumeclaims"))
        );
    }

    #[tokio::test]
    async fn retained_home_readers_do_not_block_workspace_deletion() {
        let mut workspace = workspace(workspace_id());
        bind_home(&mut workspace, "existing-home");
        let mut mock = WorkspaceDeleteMock::new("public-a", workspace.id);
        mock.referenced_pvc = "existing-home".to_owned();
        mock.pod_exists.store(false, Ordering::SeqCst);
        mock.other_pod_references_pvc.store(true, Ordering::SeqCst);
        let mock = Arc::new(mock);
        let coordinator = workspace_coordinator(mock.clone());
        assert_eq!(
            coordinator.delete_or_confirm(&workspace).await.unwrap(),
            DeleteProgress::Gone
        );
        mock.other_pod_references_pvc.store(false, Ordering::SeqCst);
        assert_eq!(
            coordinator.delete_or_confirm(&workspace).await.unwrap(),
            DeleteProgress::Gone
        );
        assert_no_pvc_delete(&mock);
    }

    fn bound_claim() -> serde_json::Value {
        serde_json::json!({
            "apiVersion": "v1", "kind": "PersistentVolumeClaim",
            "metadata": {"name": "existing-home", "namespace": "memeloop-workspace-control", "uid": "home-uid"},
            "spec": {"volumeName": "home-volume"},
            "status": {"phase": "Bound", "capacity": {"storage": "10Gi"}}
        })
    }

    #[tokio::test]
    async fn invalid_persisted_home_identity_or_capacity_makes_no_kubernetes_requests() {
        for invalid in ["namespace", "uid", "capacity", "zero"] {
            let contacted = Arc::new(AtomicBool::new(false));
            let observed = contacted.clone();
            let service = service_fn(move |_: Request<kube::client::Body>| {
                observed.store(true, Ordering::SeqCst);
                async { Ok::<_, Infallible>(not_found()) }
            });
            let mut workspace = workspace(workspace_id());
            workspace.state = WorkspaceState::Ready;
            bind_home(&mut workspace, "existing-home");
            match invalid {
                "namespace" => {
                    workspace.home_volume_binding.as_mut().unwrap().namespace = "other".to_owned()
                }
                "uid" => workspace
                    .home_volume_binding
                    .as_mut()
                    .unwrap()
                    .claim_uid
                    .clear(),
                "zero" => workspace.home_volume_binding.as_mut().unwrap().capacity_gib = 0,
                _ => workspace.template.resources.disk_gib = 100,
            }
            let builder =
                workspace_coordinator(Arc::new(WorkspaceDeleteMock::new("public-a", workspace.id)))
                    .builder;
            let coordinator =
                KubernetesCoordinator::new(kube::Client::new(service, "default"), builder);
            assert!(matches!(
                coordinator.reconcile(&workspace).await,
                Err(ReconcileError::Build(_))
            ));
            assert!(!contacted.load(Ordering::SeqCst));
        }
    }

    #[tokio::test]
    async fn invalid_home_binding_fails_before_any_mutation_or_generated_pvc_lookup() {
        let mut invalid_claims = vec![None];
        for (pointer, value) in [
            ("/metadata/uid", serde_json::json!("replacement-uid")),
            ("/metadata/uid", serde_json::Value::Null),
            ("/status/phase", serde_json::json!("Pending")),
            ("/status/phase", serde_json::json!("Lost")),
            ("/status/phase", serde_json::json!("Unknown")),
            ("/status/capacity/storage", serde_json::json!("9Gi")),
            ("/status/capacity/storage", serde_json::json!("invalid")),
            ("/status/capacity", serde_json::Value::Null),
            ("/status", serde_json::Value::Null),
        ] {
            let mut claim = bound_claim();
            *claim.pointer_mut(pointer).unwrap() = value;
            invalid_claims.push(Some(claim));
        }
        let mut terminating = bound_claim();
        terminating["metadata"]["deletionTimestamp"] = serde_json::json!("2026-10-04T00:00:00Z");
        invalid_claims.push(Some(terminating));
        for claim in invalid_claims {
            let requests = Arc::new(Mutex::new(Vec::new()));
            let recorded = requests.clone();
            let service = service_fn(move |request: Request<kube::client::Body>| {
                recorded
                    .lock()
                    .unwrap()
                    .push((request.method().clone(), request.uri().path().to_owned()));
                let response = claim
                    .clone()
                    .map_or_else(not_found, |claim| json_response(StatusCode::OK, claim));
                async move { Ok::<_, Infallible>(response) }
            });
            let mut workspace = workspace(workspace_id());
            workspace.state = WorkspaceState::Ready;
            bind_home(&mut workspace, "existing-home");
            let builder =
                workspace_coordinator(Arc::new(WorkspaceDeleteMock::new("public-a", workspace.id)))
                    .builder;
            let coordinator =
                KubernetesCoordinator::new(kube::Client::new(service, "default"), builder);
            assert!(matches!(
                coordinator.reconcile(&workspace).await,
                Err(ReconcileError::InvalidHomeVolumeBinding { .. })
            ));
            assert_eq!(*requests.lock().unwrap(), [(Method::GET, "/api/v1/namespaces/memeloop-workspace-control/persistentvolumeclaims/existing-home".to_owned())]);
        }
    }

    #[tokio::test]
    async fn bound_home_reconcile_checks_again_before_patch_and_immutable_recreation() {
        use http_body_util::BodyExt;
        use std::sync::atomic::AtomicUsize;

        for (bound, fail_on_check) in [
            (false, None),
            (true, None),
            (true, Some(2)),
            (true, Some(3)),
        ] {
            let requests = Arc::new(Mutex::new(Vec::new()));
            let recorded = requests.clone();
            let claim_reads = Arc::new(AtomicUsize::new(0));
            let read_count = claim_reads.clone();
            let service = service_fn(move |request: Request<kube::client::Body>| {
                let recorded = recorded.clone();
                let read_count = read_count.clone();
                async move {
                    let method = request.method().clone();
                    let path = request.uri().path().to_owned();
                    recorded
                        .lock()
                        .unwrap()
                        .push((method.clone(), path.clone()));
                    if path.contains("persistentvolumeclaims") {
                        assert_eq!(method, Method::GET);
                        if !bound {
                            assert!(path.ends_with("/workspace-data-w-8000000000000001-0"));
                            return Ok::<_, Infallible>(not_found());
                        }
                        assert!(path.ends_with("/existing-home"));
                        let check = read_count.fetch_add(1, Ordering::SeqCst) + 1;
                        return Ok::<_, Infallible>(if fail_on_check == Some(check) {
                            not_found()
                        } else {
                            let mut claim = bound_claim();
                            claim["status"]["capacity"]["storage"] = serde_json::json!("10240Mi");
                            json_response(StatusCode::OK, claim)
                        });
                    }
                    if method == Method::GET {
                        return Ok(not_found());
                    }
                    assert_eq!(method, Method::PATCH, "unexpected mutation: {path}");
                    let bytes = request.into_body().collect().await.unwrap().to_bytes();
                    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    if path.contains("/statefulsets/") {
                        if bound {
                            assert!(body["spec"].get("volumeClaimTemplates").is_none());
                            assert!(
                                body["spec"]["template"]["spec"]["volumes"]
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .any(|volume| {
                                        volume["name"] == "workspace-data"
                                            && volume["persistentVolumeClaim"]["claimName"]
                                                == "existing-home"
                                    })
                            );
                        } else {
                            assert_eq!(
                                body["spec"]["volumeClaimTemplates"][0]["metadata"]["name"],
                                "workspace-data"
                            );
                        }
                        if fail_on_check == Some(3) {
                            return Ok(json_response(
                                StatusCode::UNPROCESSABLE_ENTITY,
                                serde_json::json!({
                                    "apiVersion": "v1", "kind": "Status", "status": "Failure",
                                    "code": 422, "reason": "Invalid", "message": "updates to statefulset spec are forbidden"
                                }),
                            ));
                        }
                    }
                    Ok(json_response(StatusCode::OK, body))
                }
            });
            let mut workspace = workspace(workspace_id());
            workspace.state = WorkspaceState::Ready;
            if bound {
                bind_home(&mut workspace, "existing-home");
            }
            let builder =
                workspace_coordinator(Arc::new(WorkspaceDeleteMock::new("public-a", workspace.id)))
                    .builder;
            let coordinator =
                KubernetesCoordinator::new(kube::Client::new(service, "default"), builder);
            let result = coordinator.reconcile(&workspace).await;
            if fail_on_check.is_some() {
                assert!(matches!(
                    result,
                    Err(ReconcileError::InvalidHomeVolumeBinding { .. })
                ));
            } else {
                result.unwrap();
            }
            let requests = requests.lock().unwrap();
            let stateful_patches = requests
                .iter()
                .filter(|(method, path)| {
                    *method == Method::PATCH && path.contains("/statefulsets/")
                })
                .count();
            assert_eq!(stateful_patches, usize::from(fail_on_check != Some(2)));
            assert!(requests.iter().all(|(method, _)| *method != Method::DELETE));
            if bound {
                assert!(
                    requests
                        .iter()
                        .all(|(_, path)| !path.contains("/persistentvolumeclaims/workspace-data-"))
                );
            }
        }
    }

    #[tokio::test]
    async fn unpersisted_custom_home_blocks_stateful_set_mutation_but_default_homes_reconcile() {
        use http_body_util::BodyExt;

        for claim_name in [
            Some("custom-home-20gi"),
            Some("workspace-data-w-8000000000000001-0"),
            None,
        ] {
            let requests = Arc::new(Mutex::new(Vec::new()));
            let recorded = requests.clone();
            let mut workspace = workspace(workspace_id());
            workspace.state = WorkspaceState::Ready;
            workspace.template.resources.disk_gib = 100;
            let workspace_id = workspace.id;
            let service = service_fn(move |request: Request<kube::client::Body>| {
                let recorded = recorded.clone();
                async move {
                    let method = request.method().clone();
                    let path = request.uri().path().to_owned();
                    recorded
                        .lock()
                        .unwrap()
                        .push((method.clone(), path.clone()));
                    if method == Method::GET && path.contains("/statefulsets/") {
                        let volumes = claim_name.into_iter().map(|name| serde_json::json!({
                            "name": "workspace-data", "persistentVolumeClaim": {"claimName": name}
                        })).collect::<Vec<_>>();
                        return Ok::<_, Infallible>(json_response(
                            StatusCode::OK,
                            serde_json::json!({
                                "apiVersion": "apps/v1", "kind": "StatefulSet",
                                "metadata": {"name": "w-8000000000000001", "namespace": "memeloop-workspace-control", "labels": ownership_labels("public-a", workspace_id)},
                                "spec": {
                                    "replicas": 1,
                                    "selector": {"matchLabels": ownership_labels("public-a", workspace_id)},
                                    "template": {"spec": {"containers": [{"name": "workspace", "image": "example/workspace:1"}], "volumes": volumes}}
                                }
                            }),
                        ));
                    }
                    if method == Method::GET {
                        return Ok(not_found());
                    }
                    assert_eq!(method, Method::PATCH, "unexpected mutation: {path}");
                    let bytes = request.into_body().collect().await.unwrap().to_bytes();
                    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    if path.contains("/statefulsets/") {
                        assert_ne!(claim_name, Some("custom-home-20gi"));
                        assert_eq!(
                            body["spec"]["volumeClaimTemplates"][0]["spec"]["resources"]["requests"]
                                ["storage"],
                            "100Gi"
                        );
                    }
                    Ok(json_response(StatusCode::OK, body))
                }
            });
            let builder =
                workspace_coordinator(Arc::new(WorkspaceDeleteMock::new("public-a", workspace.id)))
                    .builder;
            let coordinator =
                KubernetesCoordinator::new(kube::Client::new(service, "default"), builder);
            let result = coordinator.reconcile(&workspace).await;
            let blocked = claim_name == Some("custom-home-20gi");
            if blocked {
                assert!(
                    matches!(result, Err(ReconcileError::UnpersistedHomeVolumeBinding { claim_name }) if claim_name == "custom-home-20gi")
                );
            } else {
                result.unwrap();
            }
            let requests = requests.lock().unwrap();
            assert_eq!(
                requests
                    .iter()
                    .filter(|(method, path)| *method == Method::PATCH
                        && path.contains("/statefulsets/"))
                    .count(),
                usize::from(!blocked)
            );
            assert!(requests.iter().all(|(method, _)| *method != Method::DELETE));
            if blocked {
                assert!(
                    requests
                        .iter()
                        .all(|(_, path)| !path.contains("/persistentvolumeclaims/"))
                );
            }
        }
    }

    struct WorkspaceDeleteMock {
        referenced_pvc: String,
        pod_owner: Option<String>,
        workspace_id: Uuid,
        pod_exists: AtomicBool,
        target_pod_appears_in_list: AtomicBool,
        other_pod_references_pvc: AtomicBool,
        pvc_exists: AtomicBool,
        config_exists: AtomicBool,
        requests: Mutex<Vec<(String, String)>>,
    }

    impl WorkspaceDeleteMock {
        fn new(pod_owner: &str, workspace_id: Uuid) -> Self {
            Self {
                referenced_pvc: "workspace-data-w-8000000000000001-0".to_owned(),
                pod_owner: Some(pod_owner.to_owned()),
                workspace_id,
                pod_exists: AtomicBool::new(true),
                target_pod_appears_in_list: AtomicBool::new(false),
                other_pod_references_pvc: AtomicBool::new(false),
                pvc_exists: AtomicBool::new(true),
                config_exists: AtomicBool::new(false),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn without_pod_labels(workspace_id: Uuid) -> Self {
            Self {
                referenced_pvc: "workspace-data-w-8000000000000001-0".to_owned(),
                pod_owner: None,
                workspace_id,
                pod_exists: AtomicBool::new(true),
                target_pod_appears_in_list: AtomicBool::new(false),
                other_pod_references_pvc: AtomicBool::new(false),
                pvc_exists: AtomicBool::new(true),
                config_exists: AtomicBool::new(false),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn target_pod(&self) -> serde_json::Value {
            let mut metadata = serde_json::json!({
                "name": "w-8000000000000001-0",
                "namespace": "memeloop-workspace-control",
            });
            if let Some(owner) = &self.pod_owner {
                metadata["labels"] = ownership_labels(owner, self.workspace_id);
            }
            serde_json::json!({
                "apiVersion": "v1",
                "kind": "Pod",
                "metadata": metadata,
            })
        }

        fn response(&self, method: &Method, uri: &Uri) -> Response<Body> {
            let path = uri.path();
            self.requests
                .lock()
                .unwrap()
                .push((method.to_string(), uri.to_string()));
            let namespace_path = "/api/v1/namespaces/memeloop-workspace-control";
            let pods_path = "/api/v1/namespaces/memeloop-workspace-control/pods";
            let target_pod_path =
                "/api/v1/namespaces/memeloop-workspace-control/pods/w-8000000000000001-0";
            let pvc_path = "/api/v1/namespaces/memeloop-workspace-control/persistentvolumeclaims/workspace-data-w-8000000000000001-0";
            let config_path = "/api/v1/namespaces/memeloop-workspace-control/configmaps/w-8000000000000001-config";
            if path == config_path {
                if method == Method::GET && self.config_exists.load(Ordering::SeqCst) {
                    return json_response(
                        StatusCode::OK,
                        serde_json::json!({
                            "apiVersion": "v1", "kind": "ConfigMap",
                            "metadata": {"name": "w-8000000000000001-config", "labels": ownership_labels("public-a", self.workspace_id)}
                        }),
                    );
                }
                if method == Method::DELETE {
                    self.config_exists.store(false, Ordering::SeqCst);
                    return success();
                }
            }
            if method == Method::GET && path == namespace_path {
                return json_response(
                    StatusCode::OK,
                    serde_json::json!({
                        "apiVersion": "v1",
                        "kind": "Namespace",
                        "metadata": {
                            "name": "memeloop-workspace-control",
                            "labels": {
                                "workspace.memeloop.dev/owner-installation": "public-a",
                            },
                        },
                    }),
                );
            }
            if method == Method::GET && path == target_pod_path {
                return if self.pod_exists.load(Ordering::SeqCst) {
                    json_response(StatusCode::OK, self.target_pod())
                } else {
                    not_found()
                };
            }
            if method == Method::GET && path == pods_path {
                let mut items = Vec::new();
                if self.pod_exists.load(Ordering::SeqCst)
                    || self.target_pod_appears_in_list.load(Ordering::SeqCst)
                {
                    items.push(self.target_pod());
                }
                if self.other_pod_references_pvc.load(Ordering::SeqCst) {
                    items.push(serde_json::json!({
                        "apiVersion": "v1",
                        "kind": "Pod",
                        "metadata": {
                            "name": "unrelated-pod",
                            "namespace": "memeloop-workspace-control",
                        },
                        "spec": {
                            "volumes": [{
                                "name": "workspace-data",
                                "persistentVolumeClaim": {
                                    "claimName": self.referenced_pvc,
                                },
                            }],
                        },
                    }));
                }
                return json_response(
                    StatusCode::OK,
                    serde_json::json!({
                        "apiVersion": "v1",
                        "kind": "PodList",
                        "metadata": {"resourceVersion": "1"},
                        "items": items,
                    }),
                );
            }
            if method == Method::GET && path == pvc_path {
                return if self.pvc_exists.load(Ordering::SeqCst) {
                    json_response(
                        StatusCode::OK,
                        serde_json::json!({
                            "apiVersion": "v1",
                            "kind": "PersistentVolumeClaim",
                            "metadata": {
                                "name": "workspace-data-w-8000000000000001-0",
                                "namespace": "memeloop-workspace-control",
                                "labels": ownership_labels("public-a", self.workspace_id),
                            },
                            "spec": {"accessModes": ["ReadWriteOnce"], "resources": {}},
                        }),
                    )
                } else {
                    not_found()
                };
            }
            if method == Method::DELETE && path == pvc_path {
                self.pvc_exists.store(false, Ordering::SeqCst);
                return success();
            }
            if method == Method::GET
                && matches!(
                    path,
                    "/apis/networking.k8s.io/v1/namespaces/memeloop-workspace-control/ingresses"
                        | "/apis/networking.k8s.io/v1/namespaces/memeloop-workspace-control/networkpolicies"
                        | "/api/v1/namespaces/memeloop-workspace-control/services"
                )
            {
                return json_response(
                    StatusCode::OK,
                    serde_json::json!({
                        "apiVersion": "v1",
                        "kind": "List",
                        "metadata": {"resourceVersion": "1"},
                        "items": [],
                    }),
                );
            }
            if method == Method::GET {
                return not_found();
            }
            panic!("unexpected Kubernetes request: {method} {path}")
        }
    }

    fn workspace_coordinator(mock: Arc<WorkspaceDeleteMock>) -> KubernetesCoordinator {
        let service = service_fn(move |request: Request<kube::client::Body>| {
            let mock = mock.clone();
            async move { Ok::<_, Infallible>(mock.response(request.method(), request.uri())) }
        });
        KubernetesCoordinator::new(
            kube::Client::new(service, "default"),
            ResourceBuilder {
                installation_id: "public-a".parse().unwrap(),
                ttyd_image: "example/ttyd:1".to_owned(),
                buildkit_image: "moby/buildkit:test".to_owned(),
                ttyd_mtls: None,
                higress_namespace: "higress-system".to_owned(),
                higress_pod_labels: BTreeMap::new(),
                higress_source_cidrs: Vec::new(),
                internet_egress: Some(
                    InternetEgressConfig::new(
                        "kube-system".to_owned(),
                        BTreeMap::from([("k8s-app".to_owned(), "kube-dns".to_owned())]),
                        Vec::new(),
                    )
                    .unwrap(),
                ),
                jump_host_namespace: "access".to_owned(),
                jump_host_pod_labels: BTreeMap::new(),
                storage_class_name: None,
                scratch_storage_class_name: None,
                web_shell_domain: None,
                port_mapping_domain: None,
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

    fn assert_no_pvc_delete(mock: &WorkspaceDeleteMock) {
        assert!(mock.requests.lock().unwrap().iter().all(|(method, path)| {
            method != Method::DELETE.as_str()
                || path
                    .trim_end_matches('?')
                    != "/api/v1/namespaces/memeloop-workspace-control/persistentvolumeclaims/workspace-data-w-8000000000000001-0"
        }));
    }

    fn assert_pod_list_has_no_label_selector(mock: &WorkspaceDeleteMock) {
        let requests = mock.requests.lock().unwrap();
        let pod_list_uris = requests
            .iter()
            .filter(|(method, uri)| {
                method == Method::GET.as_str()
                    && uri.split('?').next()
                        == Some("/api/v1/namespaces/memeloop-workspace-control/pods")
            })
            .map(|(_, uri)| uri.clone())
            .collect::<Vec<_>>();
        assert_eq!(pod_list_uris.len(), 1);
        assert!(
            pod_list_uris
                .iter()
                .all(|uri| !uri.contains("labelSelector="))
        );
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
