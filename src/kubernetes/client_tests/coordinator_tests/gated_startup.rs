use super::*;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{sync::atomic::AtomicUsize, time::Duration};
use tokio::sync::{Notify, Semaphore};

mod approval_create;
mod disabled_identity_overlap;
mod prerequisite_overlap;
mod route_overlap;
mod worker_capacity;

struct GateMock {
    objects: Mutex<BTreeMap<String, Value>>,
    patches: Mutex<Vec<(String, Value)>>,
    writes: Mutex<Vec<(String, Value)>>,
    events: Mutex<Vec<(String, String)>>,
    files_pending: Notify,
    files_applied: Notify,
    routes_applied: Notify,
    approval_creates: AtomicUsize,
    lose_approval_response: AtomicBool,
    hold_files: AtomicBool,
    release_files: Semaphore,
    workload_applied: Notify,
    workload_reads: AtomicUsize,
    approval_reads: AtomicUsize,
    fail_suffix: Mutex<Option<String>>,
    fail_request: Mutex<Option<(Method, String)>>,
    hold_request: Mutex<Option<(Method, String)>>,
    request_started: Notify,
    release_request: Semaphore,
}

impl Default for GateMock {
    fn default() -> Self {
        Self {
            objects: Mutex::default(),
            patches: Mutex::default(),
            writes: Mutex::default(),
            events: Mutex::default(),
            files_pending: Notify::new(),
            files_applied: Notify::new(),
            routes_applied: Notify::new(),
            approval_creates: AtomicUsize::new(0),
            lose_approval_response: AtomicBool::new(false),
            hold_files: AtomicBool::default(),
            release_files: Semaphore::new(0),
            workload_applied: Notify::new(),
            workload_reads: AtomicUsize::new(0),
            approval_reads: AtomicUsize::new(0),
            fail_suffix: Mutex::default(),
            fail_request: Mutex::default(),
            hold_request: Mutex::default(),
            request_started: Notify::new(),
            release_request: Semaphore::new(0),
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

    async fn create_approval<B>(&self, collection: &str, request: Request<B>) -> Response<Body>
    where
        B: axum::body::HttpBody<Data = axum::body::Bytes> + Send,
        B::Error: std::fmt::Debug,
    {
        assert!(
            collection.starts_with("/api/v1/namespaces/") && collection.ends_with("/configmaps")
        );
        let mut body: Value =
            serde_json::from_slice(&request.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let name = body["metadata"]["name"].as_str().unwrap().to_owned();
        assert!(name.ends_with("-startup-gate"));
        assert_eq!(body["kind"], "ConfigMap");
        let path = format!("{collection}/{name}");
        let attempt = self.approval_creates.fetch_add(1, Ordering::SeqCst) + 1;
        if self
            .fail_suffix
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|suffix| path.ends_with(suffix))
        {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({
                    "apiVersion":"v1", "kind":"Status", "status":"Failure", "code":500,
                    "reason":"InternalError", "message":"injected approval create failure"
                }),
            );
        }
        body["metadata"]["uid"] = json!(format!("{name}-uid-{attempt}"));
        {
            let mut objects = self.objects.lock().unwrap();
            let std::collections::btree_map::Entry::Vacant(entry) = objects.entry(path.clone())
            else {
                return json_response(
                    StatusCode::CONFLICT,
                    json!({
                        "apiVersion":"v1", "kind":"Status", "status":"Failure", "code":409,
                        "reason":"AlreadyExists", "message":"approval already exists"
                    }),
                );
            };
            entry.insert(body.clone());
        }
        self.writes.lock().unwrap().push((path, body.clone()));
        if self.lose_approval_response.swap(false, Ordering::SeqCst) {
            let error = std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "injected response loss after approval persistence",
            );
            return Response::builder()
                .status(StatusCode::CREATED)
                .body(Body::from_stream(futures::stream::once(async move {
                    Err::<axum::body::Bytes, _>(error)
                })))
                .unwrap();
        }
        json_response(StatusCode::CREATED, body)
    }

    async fn respond<B>(&self, request: Request<B>) -> Response<Body>
    where
        B: axum::body::HttpBody<Data = axum::body::Bytes> + Send,
        B::Error: std::fmt::Debug,
    {
        let method = request.method().clone();
        let path = request.uri().path().to_owned();
        self.events
            .lock()
            .unwrap()
            .push((method.to_string(), path.clone()));
        let held = self.hold_request.lock().unwrap().clone();
        if held
            .as_ref()
            .is_some_and(|(held_method, held_path)| method == *held_method && path == *held_path)
        {
            self.request_started.notify_one();
            self.release_request.acquire().await.unwrap().forget();
        }
        if self
            .fail_request
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|(failed_method, failed_path)| {
                method == *failed_method && path == *failed_path
            })
        {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({
                    "apiVersion":"v1", "kind":"Status", "status":"Failure", "code":500,
                    "reason":"InternalError", "message":"injected request failure"
                }),
            );
        }
        if method == Method::GET {
            if path.ends_with("-startup-gate") {
                self.approval_reads.fetch_add(1, Ordering::SeqCst);
            }
            if path.contains("/statefulsets/") {
                self.workload_reads.fetch_add(1, Ordering::SeqCst);
            }
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
        if method == Method::POST {
            return self.create_approval(&path, request).await;
        }
        assert_eq!(method, Method::PATCH);
        let metadata_only = request.headers().get("accept").is_some_and(|value| {
            value == "application/json;as=PartialObjectMetadata;g=meta.k8s.io;v=v1"
        });
        if path.ends_with("-files-config") {
            assert!(
                metadata_only,
                "file apply must request only response metadata"
            );
            assert_eq!(
                request.headers()["content-type"],
                "application/apply-patch+yaml"
            );
            let parameters =
                url::form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes())
                    .into_owned()
                    .collect::<Vec<(String, String)>>();
            assert_eq!(
                parameters,
                vec![(
                    "fieldManager".to_owned(),
                    "memeloop-workspace-control".to_owned()
                )],
                "preserve the SSA field manager without enabling force"
            );
        }
        if path.ends_with("-files-config") && self.hold_files.load(Ordering::SeqCst) {
            self.files_pending.notify_one();
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
        let mut body: Value =
            serde_json::from_slice(&request.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        self.patches
            .lock()
            .unwrap()
            .push((path.clone(), body.clone()));
        self.writes
            .lock()
            .unwrap()
            .push((path.clone(), body.clone()));
        body["metadata"]["uid"] = json!(format!("{}-uid", path.rsplit('/').next().unwrap()));
        self.objects
            .lock()
            .unwrap()
            .insert(path.clone(), body.clone());
        self.events
            .lock()
            .unwrap()
            .push(("APPLIED".to_owned(), path.clone()));
        if path.ends_with("-files-config") {
            self.files_applied.notify_one();
        }
        if path.ends_with("-web-shell") {
            self.routes_applied.notify_one();
        }
        if path.contains("/statefulsets/") {
            self.workload_applied.notify_one();
        }
        let response = if metadata_only {
            json!({
                "apiVersion":"meta.k8s.io/v1",
                "kind":"PartialObjectMetadata",
                "metadata":body["metadata"]
            })
        } else {
            body
        };
        json_response(StatusCode::OK, response)
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
    assert_eq!(mock.workload_reads.load(Ordering::SeqCst), 1);
    let before = mock.object("/statefulsets/w-8000000000000001").unwrap();
    assert_gated(&before);
    assert_eq!(
        before["metadata"]["annotations"]["workspace.memeloop.dev/file-encoding"],
        "dictionary-v1"
    );
    assert!(mock.object("-startup-gate").is_none());
    assert!(mock.object("-files-config").is_none());
    assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 0);
    mock.release_files.add_permits(1);
    task.await.unwrap().unwrap();
    let gate = mock.object("-startup-gate").unwrap();
    assert_eq!(gate["immutable"], true);
    assert_eq!(gate["data"]["approved"], "true");
    for (key, value) in ownership_labels("public-a", workspace.id)
        .as_object()
        .unwrap()
    {
        assert_eq!(&gate["metadata"]["labels"][key], value);
    }
    {
        let patches = mock.writes.lock().unwrap();
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
    assert_eq!(mock.approval_reads.load(Ordering::SeqCst), 1);
    assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 1);
    // An already validated immutable approval must not be PATCHed again.
    *mock.fail_suffix.lock().unwrap() = Some("-startup-gate".to_owned());
    workspace.state = WorkspaceState::Ready;
    coordinator.reconcile(&workspace).await.unwrap();
    assert_eq!(mock.workload_reads.load(Ordering::SeqCst), 2);
    assert_eq!(
        mock.approval_reads.load(Ordering::SeqCst),
        2,
        "approved reconciliation needs only its full validation GET, not another metadata GET"
    );
    assert_eq!(
        mock.patches
            .lock()
            .unwrap()
            .iter()
            .filter(|(path, _)| path.ends_with("-startup-gate"))
            .count(),
        0
    );
    assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 1);
    assert_eq!(mock.object("-startup-gate").unwrap(), gate);
    let after = mock.object("/statefulsets/w-8000000000000001").unwrap();
    assert_gated(&after);
    assert_eq!(
        before["metadata"]["annotations"],
        after["metadata"]["annotations"]
    );
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
    tokio::time::timeout(Duration::from_secs(3), mock.routes_applied.notified())
        .await
        .expect("routes may finish before the pending file upload fails");
    mock.release_files.add_permits(1);
    assert!(matches!(
        task.await.unwrap(),
        Err(ReconcileError::Kubernetes(_))
    ));
    assert!(mock.object("-startup-gate").is_none());
    assert!(mock.object("-web-shell").is_some());
    assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 0);
    assert!(
        !mock
            .patches
            .lock()
            .unwrap()
            .iter()
            .any(|(path, _)| path.ends_with("-startup-gate"))
    );
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
        let coordinator = Arc::new(coordinator(mock.clone()));
        if suffix == "-web-shell" {
            let name = coordinator
                .builder
                .runtime_names(&workspace)
                .unwrap()
                .resources
                .web_shell_ingress;
            *mock.hold_request.lock().unwrap() = Some((
                Method::PATCH,
                format!(
                    "/apis/networking.k8s.io/v1/namespaces/{}/ingresses/{name}",
                    workspace.runtime.namespace()
                ),
            ));
        }
        let task = tokio::spawn({
            let (coordinator, workspace) = (coordinator.clone(), workspace.clone());
            async move { coordinator.reconcile(&workspace).await }
        });
        if suffix == "-web-shell" {
            wait_for_workload(&mock).await;
            mock.release_request.add_permits(1);
        }
        assert!(
            tokio::time::timeout(Duration::from_secs(3), task)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert_gated(&mock.object("/statefulsets/w-8000000000000001").unwrap());
        assert!(mock.object("-startup-gate").is_none());
        assert_eq!(
            mock.approval_creates.load(Ordering::SeqCst),
            usize::from(suffix == "-startup-gate")
        );
        *mock.fail_suffix.lock().unwrap() = None;
        *mock.hold_request.lock().unwrap() = None;
        coordinator.reconcile(&workspace).await.unwrap();
        assert!(mock.object("-startup-gate").is_some());
    }
}

#[tokio::test]
async fn failed_access_or_isolation_prevents_early_workload_creation() {
    for suffix in ["-admin", "-ingress"] {
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
        assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn foreign_or_malformed_approval_is_never_reused_or_overwritten() {
    for previously_approved in [false, true] {
        for (foreign, approved, immutable) in [
            (true, "true", true),
            (false, "false", true),
            (false, "true", false),
        ] {
            let mut workspace = workspace(workspace_id());
            workspace.state = WorkspaceState::Provisioning;
            let mock = Arc::new(GateMock::default());
            let coordinator = coordinator(mock.clone());
            if previously_approved {
                coordinator.reconcile(&workspace).await.unwrap();
                workspace.state = WorkspaceState::Ready;
            }
            let workload = mock.object("/statefulsets/w-8000000000000001");
            if !previously_approved {
                assert!(workload.is_none());
            }
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
            let namespace_path = format!("/api/v1/namespaces/{}", workspace.runtime.namespace());
            let patches = mock
                .patches
                .lock()
                .unwrap()
                .iter()
                .filter(|(path, _)| path != &namespace_path)
                .count();
            let error = coordinator.reconcile(&workspace).await.unwrap_err();
            if foreign {
                assert!(matches!(error, ReconcileError::Ownership(_)));
            } else {
                assert!(matches!(error, ReconcileError::RuntimeIdentityMismatch));
            }
            assert_eq!(mock.object("/statefulsets/w-8000000000000001"), workload);
            assert_eq!(mock.objects.lock().unwrap().get(&path), Some(&gate));
            assert_eq!(
                mock.patches
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(path, _)| path != &namespace_path)
                    .count(),
                patches,
                "invalid approvals must be rejected before applying workspace resources"
            );
        }
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
    let text = "repeated instruction\\n".repeat(8192);
    reconcile_large_file(&coordinator, &workspace, &text)
        .await
        .unwrap();
    assert_eq!(
        mock.object("-files-config").unwrap()["data"]["large-file"],
        text
    );
}

async fn reconcile_large_file(
    coordinator: &KubernetesCoordinator,
    workspace: &Workspace,
    text: &str,
) -> Result<(), ReconcileError> {
    let mut desired = coordinator.builder.build(workspace).unwrap();
    let data = desired.injections.file_config_map.data.as_mut().unwrap();
    data.insert("large-file".to_owned(), text.to_owned());
    data.insert(
        "workspace-files.json".to_owned(),
        json!({"version":1,"files":[{
            "object":"config_map","kind":"config_file","data_key":"large-file",
            "target":"/workspace/catalog.json","mode":416
        }]})
        .to_string(),
    );
    coordinator
        .reconcile_with_injections(
            workspace,
            &crate::workspaces::ResolvedPlacement::default(),
            desired.injections,
            desired.ssh_identity,
        )
        .await
}

#[tokio::test]
async fn new_workspaces_keep_dictionary_transport_after_becoming_ready() {
    let mut workspace = workspace(workspace_id());
    workspace.state = WorkspaceState::Provisioning;
    let mock = Arc::new(GateMock::default());
    let coordinator = coordinator(mock.clone());
    let text = "repeated instruction\\n".repeat(8192);
    reconcile_large_file(&coordinator, &workspace, &text)
        .await
        .unwrap();
    let before = mock.object("/statefulsets/w-8000000000000001").unwrap();
    let files = mock.object("-files-config").unwrap();
    let manifest: Value =
        serde_json::from_str(files["data"]["workspace-files.json"].as_str().unwrap()).unwrap();
    assert_eq!(manifest["files"][0]["encoding"], "dictionary_v1");
    assert_eq!(manifest["files"][0]["decoded_bytes"], text.len());
    assert!(files["data"]["large-file"].as_str().unwrap().len() < text.len() / 2);
    workspace.state = WorkspaceState::Ready;
    reconcile_large_file(&coordinator, &workspace, &text)
        .await
        .unwrap();
    assert_eq!(files, mock.object("-files-config").unwrap());
    let after = mock.object("/statefulsets/w-8000000000000001").unwrap();
    assert_eq!(before["spec"]["template"], after["spec"]["template"]);
    assert_eq!(
        after["metadata"]["annotations"]["workspace.memeloop.dev/file-encoding"],
        "dictionary-v1"
    );
    assert!(
        after["spec"]["template"]["metadata"]["annotations"]
            .get("workspace.memeloop.dev/file-encoding")
            .is_none()
    );
}

#[tokio::test]
async fn previously_gated_workspaces_keep_unencoded_files_and_the_same_pod_template() {
    let mut workspace = workspace(workspace_id());
    workspace.state = WorkspaceState::Provisioning;
    let mock = Arc::new(GateMock::default());
    let coordinator = coordinator(mock.clone());
    coordinator.reconcile(&workspace).await.unwrap();
    let before = {
        let mut objects = mock.objects.lock().unwrap();
        let (_, old) = objects
            .iter_mut()
            .find(|(path, _)| path.contains("/statefulsets/"))
            .unwrap();
        old["metadata"]["annotations"]
            .as_object_mut()
            .unwrap()
            .remove("workspace.memeloop.dev/file-encoding");
        old.clone()
    };
    workspace.state = WorkspaceState::Ready;
    let text = "repeated instruction\\n".repeat(8192);
    reconcile_large_file(&coordinator, &workspace, &text)
        .await
        .unwrap();
    assert_eq!(
        mock.object("-files-config").unwrap()["data"]["large-file"],
        text
    );
    let after = mock.object("/statefulsets/w-8000000000000001").unwrap();
    assert_eq!(before["spec"]["template"], after["spec"]["template"]);
    assert!(
        after["metadata"]["annotations"]
            .get("workspace.memeloop.dev/file-encoding")
            .is_none()
    );
}

#[tokio::test]
async fn unknown_workload_encoding_does_not_overwrite_existing_files() {
    let mut workspace = workspace(workspace_id());
    workspace.state = WorkspaceState::Provisioning;
    let mock = Arc::new(GateMock::default());
    let coordinator = coordinator(mock.clone());
    coordinator.reconcile(&workspace).await.unwrap();
    let files = mock.object("-files-config").unwrap();
    {
        let mut objects = mock.objects.lock().unwrap();
        let (_, old) = objects
            .iter_mut()
            .find(|(path, _)| path.contains("/statefulsets/"))
            .unwrap();
        old["metadata"]["annotations"]["workspace.memeloop.dev/file-encoding"] = json!("unknown");
    }
    workspace.state = WorkspaceState::Ready;
    assert!(matches!(
        reconcile_large_file(&coordinator, &workspace, &"line\n".repeat(16384)).await,
        Err(ReconcileError::RuntimeIdentityMismatch)
    ));
    assert_eq!(files, mock.object("-files-config").unwrap());
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
