use super::*;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::{Notify, Semaphore};

struct GateMock {
    objects: Mutex<BTreeMap<String, Value>>,
    patches: Mutex<Vec<(String, Value)>>,
    hold_files: AtomicBool,
    release_files: Semaphore,
    workload_applied: Notify,
    fail_suffix: Mutex<Option<String>>,
}

impl Default for GateMock {
    fn default() -> Self {
        Self {
            objects: Mutex::default(),
            patches: Mutex::default(),
            hold_files: AtomicBool::default(),
            release_files: Semaphore::new(0),
            workload_applied: Notify::new(),
            fail_suffix: Mutex::default(),
        }
    }
}

impl GateMock {
    fn object(&self, suffix: &str) -> Option<Value> {
        self.objects
            .lock()
            .unwrap()
            .iter()
            .find_map(|(path, value)| path.ends_with(suffix).then(|| value.clone()))
    }

    async fn respond(&self, request: Request<kube::client::Body>) -> Response<Body> {
        let method = request.method().clone();
        let path = request.uri().path().to_owned();
        if method == Method::GET {
            if path.ends_with("/pods")
                || path.ends_with("/services")
                || path.ends_with("/ingresses")
                || path.ends_with("/networkpolicies")
            {
                return json_response(
                    StatusCode::OK,
                    json!({
                        "apiVersion":"v1", "kind":"List", "metadata":{"resourceVersion":"1"}, "items":[]
                    }),
                );
            }
            return self
                .objects
                .lock()
                .unwrap()
                .get(&path)
                .cloned()
                .map_or_else(not_found, |object| json_response(StatusCode::OK, object));
        }
        if method == Method::DELETE {
            self.objects.lock().unwrap().remove(&path);
            return success();
        }
        assert_eq!(method, Method::PATCH);
        if path.ends_with("-files-config") && self.hold_files.load(Ordering::SeqCst) {
            self.release_files.acquire().await.unwrap().forget();
        }
        let failure = self.fail_suffix.lock().unwrap().clone();
        if failure
            .as_ref()
            .is_some_and(|suffix| path.ends_with(suffix))
        {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({
                    "apiVersion":"v1", "kind":"Status", "status":"Failure", "code":500,
                    "reason":"InternalError", "message":"injected failure"
                }),
            );
        }
        let body: Value =
            serde_json::from_slice(&request.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        self.patches
            .lock()
            .unwrap()
            .push((path.clone(), body.clone()));
        self.objects
            .lock()
            .unwrap()
            .insert(path.clone(), body.clone());
        if path.contains("/statefulsets/") {
            self.workload_applied.notify_one();
        }
        json_response(StatusCode::OK, body)
    }
}

fn coordinator(mock: Arc<GateMock>) -> KubernetesCoordinator {
    let mut builder = workspace_coordinator(Arc::new(WorkspaceDeleteMock::new(
        "public-a",
        workspace_id(),
    )))
    .builder;
    builder.web_shell_domain = Some("shell.example.invalid".to_owned());
    let service = service_fn(move |request| {
        let mock = mock.clone();
        async move { Ok::<_, Infallible>(mock.respond(request).await) }
    });
    KubernetesCoordinator::new(kube::Client::new(service, "default"), builder)
}

fn assert_gated(workload: &Value) {
    let name = workload["metadata"]["annotations"]["workspace.memeloop.dev/startup-gate"]
        .as_str()
        .unwrap();
    let spec = &workload["spec"]["template"]["spec"];
    let volume = spec["volumes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "workspace-startup-gate")
        .unwrap();
    assert_eq!(volume["configMap"]["name"], name);
    assert_eq!(volume["configMap"]["optional"], false);
    assert_eq!(volume["configMap"]["items"][0]["key"], "approved");
    for container in spec["containers"]
        .as_array()
        .unwrap()
        .iter()
        .chain(spec["initContainers"].as_array().unwrap())
    {
        assert!(
            container["volumeMounts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|mount| {
                    mount["name"] == "workspace-startup-gate" && mount["readOnly"] == true
                })
        );
    }
}

async fn wait_for_workload(mock: &GateMock) {
    tokio::time::timeout(Duration::from_secs(3), mock.workload_applied.notified())
        .await
        .expect("gated workload must be applied while files are still pending");
}

#[tokio::test]
async fn new_workload_prepares_volumes_before_files_but_approval_waits_for_every_prerequisite() {
    let mut workspace = workspace(workspace_id());
    workspace.state = WorkspaceState::Provisioning;
    let mock = Arc::new(GateMock::default());
    mock.hold_files.store(true, Ordering::SeqCst);
    let coordinator = Arc::new(coordinator(mock.clone()));
    let task = tokio::spawn({
        let coordinator = coordinator.clone();
        let workspace = workspace.clone();
        async move { coordinator.reconcile(&workspace).await }
    });
    wait_for_workload(&mock).await;
    let before = mock.object("/statefulsets/w-8000000000000001").unwrap();
    assert_gated(&before);
    assert!(mock.object("-startup-gate").is_none());
    assert!(mock.object("-files-config").is_none());
    assert!(mock.object("-web-shell").is_none());
    mock.release_files.add_permits(1);
    task.await.unwrap().unwrap();
    let gate = mock.object("-startup-gate").unwrap();
    assert_eq!(gate["immutable"], true);
    assert_eq!(gate["data"]["approved"], "true");
    {
        let patches = mock.patches.lock().unwrap();
        let gate_position = patches
            .iter()
            .position(|(p, _)| p.ends_with("-startup-gate"))
            .unwrap();
        for path in startup_prerequisite_paths(&coordinator.builder, &workspace) {
            assert!(patches.iter().position(|(p, _)| p == &path).unwrap() < gate_position);
        }
        assert!(
            patches
                .iter()
                .position(|(p, _)| p.contains("/ingresses/"))
                .unwrap()
                < gate_position
        );
        assert_eq!(
            patches
                .iter()
                .filter(|(p, _)| p.contains("/statefulsets/"))
                .count(),
            1
        );
    }
    mock.hold_files.store(false, Ordering::SeqCst);
    workspace.state = WorkspaceState::Ready;
    coordinator.reconcile(&workspace).await.unwrap();
    let after = mock.object("/statefulsets/w-8000000000000001").unwrap();
    assert_eq!(
        before["spec"]["template"], after["spec"]["template"],
        "ready reconciliation must not restart the newly gated Pod"
    );
}

#[tokio::test]
async fn failed_files_keep_early_workload_blocked_and_retry_approves_the_same_template() {
    let mut workspace = workspace(workspace_id());
    workspace.state = WorkspaceState::Provisioning;
    let mock = Arc::new(GateMock::default());
    mock.hold_files.store(true, Ordering::SeqCst);
    *mock.fail_suffix.lock().unwrap() = Some("-files-config".to_owned());
    let coordinator = Arc::new(coordinator(mock.clone()));
    let task = tokio::spawn({
        let coordinator = coordinator.clone();
        let workspace = workspace.clone();
        async move { coordinator.reconcile(&workspace).await }
    });
    wait_for_workload(&mock).await;
    let before = mock.object("/statefulsets/w-8000000000000001").unwrap();
    assert_gated(&before);
    mock.release_files.add_permits(1);
    assert!(matches!(
        task.await.unwrap(),
        Err(ReconcileError::Kubernetes(_))
    ));
    assert!(mock.object("-startup-gate").is_none());
    assert!(mock.object("-web-shell").is_none());
    mock.hold_files.store(false, Ordering::SeqCst);
    *mock.fail_suffix.lock().unwrap() = None;
    coordinator.reconcile(&workspace).await.unwrap();
    assert!(mock.object("-startup-gate").is_some());
    assert_eq!(
        before["spec"]["template"],
        mock.object("/statefulsets/w-8000000000000001").unwrap()["spec"]["template"]
    );
}

#[tokio::test]
async fn failed_route_or_approval_never_releases_the_startup_gate() {
    for suffix in ["-web-shell", "-startup-gate"] {
        let mut workspace = workspace(workspace_id());
        workspace.state = WorkspaceState::Provisioning;
        let mock = Arc::new(GateMock::default());
        *mock.fail_suffix.lock().unwrap() = Some(suffix.to_owned());
        let coordinator = coordinator(mock.clone());
        assert!(coordinator.reconcile(&workspace).await.is_err());
        assert_gated(&mock.object("/statefulsets/w-8000000000000001").unwrap());
        assert!(mock.object("-startup-gate").is_none());
        *mock.fail_suffix.lock().unwrap() = None;
        coordinator.reconcile(&workspace).await.unwrap();
        assert!(mock.object("-startup-gate").is_some());
    }
}

#[tokio::test]
async fn failed_small_prerequisites_prevent_early_workload_creation() {
    for suffix in [
        "-admin",
        "-ssh-identity",
        "-config",
        "-ingress",
        "/w-8000000000000001",
    ] {
        let mut workspace = workspace(workspace_id());
        workspace.state = WorkspaceState::Provisioning;
        workspace.template.cluster_access = true;
        let mock = Arc::new(GateMock::default());
        mock.hold_files.store(true, Ordering::SeqCst);
        *mock.fail_suffix.lock().unwrap() = Some(suffix.to_owned());
        let coordinator = coordinator(mock.clone());
        tokio::time::timeout(Duration::from_secs(3), coordinator.reconcile(&workspace))
            .await
            .expect("a failed small prerequisite must cancel the pending file upload")
            .unwrap_err();
        assert!(mock.object("/statefulsets/w-8000000000000001").is_none());
        assert!(mock.object("-startup-gate").is_none());
        assert!(mock.object("-web-shell").is_none());
    }
}

#[tokio::test]
async fn foreign_or_malformed_approval_is_never_reused_or_overwritten() {
    for (foreign, approved, immutable) in [
        (true, "true", true),
        (false, "false", true),
        (false, "true", false),
    ] {
        let mut workspace = workspace(workspace_id());
        workspace.state = WorkspaceState::Provisioning;
        let mock = Arc::new(GateMock::default());
        let coordinator = coordinator(mock.clone());
        let names = coordinator
            .builder
            .runtime_names(&workspace)
            .unwrap()
            .resources;
        let path = format!(
            "/api/v1/namespaces/{}/configmaps/{}",
            workspace.runtime.namespace(),
            names.startup_gate_config_map()
        );
        let gate = json!({"apiVersion":"v1", "kind":"ConfigMap", "metadata":{
            "name":names.startup_gate_config_map(),
            "labels":ownership_labels(if foreign {"foreign"} else {"public-a"}, workspace.id)
        }, "immutable":immutable, "data":{"approved":approved}});
        mock.objects
            .lock()
            .unwrap()
            .insert(path.clone(), gate.clone());
        assert!(coordinator.reconcile(&workspace).await.is_err());
        assert!(mock.object("/statefulsets/w-8000000000000001").is_none());
        assert_eq!(mock.objects.lock().unwrap().get(&path), Some(&gate));
    }
}

#[tokio::test]
async fn preexisting_workspaces_keep_their_ungated_template() {
    let mut workspace = workspace(workspace_id());
    workspace.state = WorkspaceState::Starting;
    let mock = Arc::new(GateMock::default());
    let coordinator = coordinator(mock.clone());
    let desired = coordinator.builder.build(&workspace).unwrap();
    let path = format!(
        "/apis/apps/v1/namespaces/{}/statefulsets/{}",
        workspace.runtime.namespace(),
        desired.stateful_set.metadata.name.as_ref().unwrap()
    );
    let old = serde_json::to_value(desired.stateful_set).unwrap();
    mock.objects.lock().unwrap().insert(path, old.clone());
    coordinator.reconcile(&workspace).await.unwrap();
    assert!(mock.object("-startup-gate").is_none());
    assert_eq!(
        old["spec"]["template"],
        mock.object("/statefulsets/w-8000000000000001").unwrap()["spec"]["template"]
    );
}

#[tokio::test]
async fn deleting_a_gated_workspace_removes_approval_before_confirming_gone() {
    let mut workspace = workspace(workspace_id());
    workspace.state = WorkspaceState::Provisioning;
    let mock = Arc::new(GateMock::default());
    let coordinator = coordinator(mock.clone());
    coordinator.reconcile(&workspace).await.unwrap();
    assert!(mock.object("-startup-gate").is_some());
    workspace.state = WorkspaceState::Deleting;
    for _ in 0..30 {
        if coordinator.delete_or_confirm(&workspace).await.unwrap() == DeleteProgress::Gone {
            assert!(mock.object("-startup-gate").is_none());
            return;
        }
    }
    panic!("workspace deletion did not remove its startup gate");
}
