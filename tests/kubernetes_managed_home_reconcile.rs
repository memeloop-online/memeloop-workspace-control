use std::{
    collections::BTreeMap,
    convert::Infallible,
    sync::{Arc, Mutex},
};

use axum::{
    body::Body,
    http::{Method, Request, Response, StatusCode},
};
use http_body_util::BodyExt;
use memeloop_workspace_control::{
    kubernetes::{InternetEgressConfig, KubernetesCoordinator, ReconcileError, ResourceBuilder},
    quota::Resources,
    templates::WorkspaceTemplateSpec,
    workspace_runtime::WorkspaceRuntimeIdentity,
    workspaces::{AccessMode, Workspace, WorkspaceState},
};
use serde_json::{Value, json};
use tower::service_fn;
use uuid::Uuid;

#[tokio::test]
async fn stopped_managed_home_recreates_only_sts_and_preserves_existing_pvc() {
    let builder = builder();
    let mut workspace = workspace();
    let old_sts = serde_json::to_value(builder.build(&workspace).unwrap().stateful_set).unwrap();
    let mock = Arc::new(Mutex::new(ManagedHomeMock::new(old_sts)));
    let requests = mock.clone();
    let service = service_fn(move |request: Request<kube::client::Body>| {
        let requests = requests.clone();
        async move {
            let method = request.method().clone();
            let path = request.uri().path().to_owned();
            let bytes = request.into_body().collect().await.unwrap().to_bytes();
            let body = if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap()
            };
            let (status, response) = requests.lock().unwrap().respond(method, &path, body);
            Ok::<_, Infallible>(
                Response::builder()
                    .status(status)
                    .header("Content-Type", "application/json")
                    .body(Body::from(response.to_string()))
                    .unwrap(),
            )
        }
    });
    let coordinator = KubernetesCoordinator::new(
        kube::Client::new(service, workspace.runtime.namespace()),
        builder,
    );
    workspace.template.resources.disk_gib = 20;
    workspace.generation += 1;
    let original_pvc = mock.lock().unwrap().pvc.clone();

    assert!(matches!(
        coordinator.reconcile(&workspace).await,
        Err(ReconcileError::StatefulSetRecreationPending)
    ));
    {
        let state = mock.lock().unwrap();
        assert!(state.sts.is_none());
        assert_eq!(
            state.events,
            ["get-old-sts", "reject-vct-update", "delete-sts"]
        );
        assert_eq!(state.pvc, original_pvc);
    }

    coordinator.reconcile(&workspace).await.unwrap();
    let state = mock.lock().unwrap();
    assert_eq!(
        state.events,
        [
            "get-old-sts",
            "reject-vct-update",
            "delete-sts",
            "get-absent-sts",
            "create-sts",
            "get-pvc",
            "patch-pvc-labels"
        ]
    );
    let recreated = state.sts.as_ref().unwrap();
    assert_eq!(recreated["metadata"]["uid"], "new-sts-uid");
    assert_eq!(recreated["spec"]["replicas"], 0);
    assert_eq!(
        recreated["spec"]["volumeClaimTemplates"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let claim = &recreated["spec"]["volumeClaimTemplates"][0];
    assert_eq!(claim["metadata"]["name"], "workspace-data");
    assert_eq!(claim["spec"]["resources"]["requests"]["storage"], "20Gi");
    assert_eq!(state.pvc, original_pvc);
    assert!(workspace.home_volume_binding.is_none());
    assert_eq!(workspace.state, WorkspaceState::Stopped);
}

struct ManagedHomeMock {
    sts: Option<Value>,
    pvc: Value,
    sts_path: String,
    pvc_path: String,
    events: Vec<&'static str>,
}

impl ManagedHomeMock {
    fn new(mut sts: Value) -> Self {
        sts["metadata"]["uid"] = json!("old-sts-uid");
        let namespace = sts["metadata"]["namespace"].as_str().unwrap();
        let name = sts["metadata"]["name"].as_str().unwrap();
        let pvc_name = format!("workspace-data-{name}-0");
        let sts_path = format!("/apis/apps/v1/namespaces/{namespace}/statefulsets/{name}");
        let pvc_path = format!("/api/v1/namespaces/{namespace}/persistentvolumeclaims/{pvc_name}");
        let pvc = json!({
            "apiVersion": "v1", "kind": "PersistentVolumeClaim",
            "metadata": {"name": pvc_name, "namespace": namespace, "uid": "existing-pvc-uid",
                "labels": sts["spec"]["volumeClaimTemplates"][0]["metadata"]["labels"]},
            "spec": {"accessModes": ["ReadWriteOnce"], "volumeName": "existing-home-pv",
                "resources": {"requests": {"storage": "20Gi"}}},
            "status": {"phase": "Bound", "capacity": {"storage": "20Gi"}}
        });
        assert_eq!(sts["spec"]["replicas"], 0);
        assert_eq!(
            sts["spec"]["volumeClaimTemplates"][0]["spec"]["resources"]["requests"]["storage"],
            "100Gi"
        );
        Self {
            sts: Some(sts),
            pvc,
            sts_path,
            pvc_path,
            events: Vec::new(),
        }
    }

    fn respond(&mut self, method: Method, path: &str, body: Value) -> (StatusCode, Value) {
        if path == self.sts_path {
            return self.stateful_set(method, body);
        }
        if path.contains("/persistentvolumeclaims") {
            assert_eq!(path, self.pvc_path);
            return self.persistent_claim(method, body);
        }
        assert!(
            !path.contains("/pods"),
            "stopped reconcile must not mutate or require a Pod"
        );
        match method {
            Method::GET => missing(),
            Method::PATCH => (StatusCode::OK, body),
            _ => panic!("unexpected mutation: {method} {path}"),
        }
    }

    fn stateful_set(&mut self, method: Method, mut body: Value) -> (StatusCode, Value) {
        match method {
            Method::GET => match &self.sts {
                Some(sts) => {
                    self.events.push("get-old-sts");
                    (StatusCode::OK, sts.clone())
                }
                None => {
                    self.events.push("get-absent-sts");
                    missing()
                }
            },
            Method::PATCH => {
                assert_eq!(body["spec"]["replicas"], 0);
                assert_eq!(
                    body["spec"]["volumeClaimTemplates"][0]["spec"]["resources"]["requests"]["storage"],
                    "20Gi"
                );
                if self.sts.is_some() {
                    self.events.push("reject-vct-update");
                    return (
                        StatusCode::UNPROCESSABLE_ENTITY,
                        json!({
                            "apiVersion": "v1", "kind": "Status", "status": "Failure",
                            "code": 422, "reason": "Invalid",
                            "message": "updates to StatefulSet spec are forbidden"
                        }),
                    );
                }
                self.events.push("create-sts");
                body["metadata"]["uid"] = json!("new-sts-uid");
                self.sts = Some(body.clone());
                (StatusCode::CREATED, body)
            }
            Method::DELETE => {
                self.events.push("delete-sts");
                (
                    StatusCode::OK,
                    self.sts.take().expect("only delete the existing STS"),
                )
            }
            _ => panic!("unexpected StatefulSet method: {method}"),
        }
    }

    fn persistent_claim(&mut self, method: Method, body: Value) -> (StatusCode, Value) {
        match method {
            Method::GET => self.events.push("get-pvc"),
            Method::PATCH => {
                self.events.push("patch-pvc-labels");
                assert_eq!(
                    body,
                    json!({"metadata": {"labels": self.pvc["metadata"]["labels"]}})
                );
            }
            _ => panic!("must not delete or replace the managed PVC: {method}"),
        }
        (StatusCode::OK, self.pvc.clone())
    }
}

fn missing() -> (StatusCode, Value) {
    (
        StatusCode::NOT_FOUND,
        json!({"apiVersion": "v1", "kind": "Status", "status": "Failure",
        "code": 404, "reason": "NotFound", "message": "fixture missing"}),
    )
}

fn workspace() -> Workspace {
    let id = Uuid::parse_str("00000000-0000-7000-8000-00000001abcd").unwrap();
    let short_id = "800000000001abcd".to_owned();
    Workspace {
        id,
        runtime: WorkspaceRuntimeIdentity::new(id, &short_id).unwrap(),
        short_id,
        organization_id: Uuid::now_v7(),
        owner_id: Uuid::now_v7(),
        name: "managed-home".to_owned(),
        template_id: Some(Uuid::now_v7()),
        home_volume_binding: None,
        node_pool: "default".to_owned(),
        template: WorkspaceTemplateSpec::standard(
            "example/workspace:1",
            AccessMode::Public,
            Resources {
                cpu_millis: 2000,
                memory_mib: 4096,
                gpu_count: 0,
                disk_gib: 100,
            },
        ),
        state: WorkspaceState::Stopped,
        generation: 1,
        created_at: 1,
        updated_at: 1,
    }
}

fn builder() -> ResourceBuilder {
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
    }
}
