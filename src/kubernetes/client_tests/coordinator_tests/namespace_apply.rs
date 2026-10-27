use super::*;
use http_body_util::BodyExt;
use serde_json::{Value, json};

const NAMESPACE: &str = "/api/v1/namespaces/memeloop-workspace-control";
const WORKLOAD: &str =
    "/apis/apps/v1/namespaces/memeloop-workspace-control/statefulsets/w-8000000000000001";
const OWNER: &str = "workspace.memeloop.dev/owner-installation";
const MANAGER: &str = "app.kubernetes.io/managed-by";

fn owned_namespace() -> Value {
    // Metadata GET has no Namespace status/phase. Additional external labels are preserved.
    json!({"apiVersion":"meta.k8s.io/v1", "kind":"PartialObjectMetadata", "metadata":{
        "name":"memeloop-workspace-control", "labels":{
            (OWNER):"public-a", (MANAGER):"memeloop-workspace-control", "external.example/team":"dev"
        }
    }})
}

async fn reconcile_namespace(
    namespace: Option<Value>,
    requests: Arc<Mutex<Vec<(Method, String)>>>,
    expect_patch: Option<bool>,
) -> Result<(), ReconcileError> {
    let mut workspace = workspace(workspace_id());
    workspace.state = WorkspaceState::Provisioning;
    let builder =
        workspace_coordinator(Arc::new(WorkspaceDeleteMock::new("public-a", workspace.id))).builder;
    let namespace_boundary = Arc::new(tokio::sync::Semaphore::new(0));
    let service = service_fn(move |request: Request<kube::client::Body>| {
        let requests = requests.clone();
        let namespace = namespace.clone();
        let namespace_boundary = namespace_boundary.clone();
        async move {
            let method = request.method().clone();
            let path = request.uri().path().to_owned();
            requests
                .lock()
                .unwrap()
                .push((method.clone(), path.clone()));
            if path == NAMESPACE {
                if method == Method::GET {
                    if expect_patch == Some(false) {
                        namespace_boundary.add_permits(1);
                    }
                    return Ok::<_, Infallible>(
                        namespace
                            .map_or_else(not_found, |object| json_response(StatusCode::OK, object)),
                    );
                }
                assert_eq!(method, Method::PATCH);
                let value: Value = serde_json::from_slice(
                    &request.into_body().collect().await.unwrap().to_bytes(),
                )
                .unwrap();
                assert_eq!(
                    value["metadata"]["labels"],
                    json!({
                        (OWNER):"public-a", (MANAGER):"memeloop-workspace-control"
                    })
                );
                namespace_boundary.add_permits(1);
                return Ok(json_response(StatusCode::OK, value));
            }
            // Stop at the next real API boundary so namespace tests do not mock startup.
            assert_eq!((method, path.as_str()), (Method::GET, WORKLOAD));
            // Planning can read concurrently. Keep this artificial failure from
            // cancelling the namespace operation that these tests exercise.
            namespace_boundary.acquire().await.unwrap().forget();
            Ok(json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                json!({
                    "apiVersion":"v1", "kind":"Status", "status":"Failure",
                    "reason":"ServiceUnavailable", "code":503, "message":"stop after namespace"
                }),
            ))
        }
    });
    KubernetesCoordinator::new(kube::Client::new(service, "default"), builder)
        .reconcile(&workspace)
        .await
}

fn continued_after_namespace(result: Result<(), ReconcileError>) {
    assert!(
        matches!(result, Err(ReconcileError::Kubernetes(kube::Error::Api(error)))
        if error.code == 503 && error.message == "stop after namespace")
    );
}

#[tokio::test]
async fn satisfied_namespace_labels_skip_patch_after_the_metadata_get() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    continued_after_namespace(
        reconcile_namespace(Some(owned_namespace()), requests.clone(), Some(false)).await,
    );
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests.contains(&(Method::GET, NAMESPACE.to_owned())));
    assert!(requests.contains(&(Method::GET, WORKLOAD.to_owned())));
}

#[tokio::test]
async fn absent_namespace_or_missing_and_drifted_managed_by_still_get_patched() {
    let mut missing = owned_namespace();
    missing["metadata"]["labels"]
        .as_object_mut()
        .unwrap()
        .remove(MANAGER);
    let mut drifted = owned_namespace();
    drifted["metadata"]["labels"][MANAGER] = json!("another-manager");
    for namespace in [None, Some(missing), Some(drifted)] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        continued_after_namespace(
            reconcile_namespace(namespace, requests.clone(), Some(true)).await,
        );
        let requests = requests.lock().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|(_, path)| path == NAMESPACE)
                .cloned()
                .collect::<Vec<_>>(),
            [
                (Method::GET, NAMESPACE.to_owned()),
                (Method::PATCH, NAMESPACE.to_owned()),
            ]
        );
        assert_eq!(requests.len(), 3);
        assert!(requests.contains(&(Method::GET, WORKLOAD.to_owned())));
    }
}

#[tokio::test]
async fn foreign_unowned_or_workspace_labeled_namespace_is_rejected_before_any_patch() {
    let mut foreign = owned_namespace();
    foreign["metadata"]["labels"][OWNER] = json!("another-installation");
    let mut unowned = owned_namespace();
    unowned["metadata"]["labels"]
        .as_object_mut()
        .unwrap()
        .remove(OWNER);
    let mut per_workspace = owned_namespace();
    per_workspace["metadata"]["labels"]["workspace.memeloop.dev/workspace-id"] =
        json!(workspace_id());
    for namespace in [foreign, unowned, per_workspace] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        assert!(matches!(
            reconcile_namespace(Some(namespace), requests.clone(), None).await,
            Err(ReconcileError::Ownership(_))
        ));
        let requests = requests.lock().unwrap();
        assert!(requests.contains(&(Method::GET, NAMESPACE.to_owned())));
        assert!(requests.iter().all(|(method, path)| {
            method == Method::GET && matches!(path.as_str(), NAMESPACE | WORKLOAD)
        }));
    }
}

#[tokio::test]
async fn terminating_namespace_keeps_the_existing_patch_path_even_with_satisfied_labels() {
    let mut namespace = owned_namespace();
    namespace["metadata"]["deletionTimestamp"] = json!("2026-10-23T00:00:00Z");
    let requests = Arc::new(Mutex::new(Vec::new()));
    continued_after_namespace(
        reconcile_namespace(Some(namespace), requests.clone(), Some(true)).await,
    );
    let requests = requests.lock().unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|(_, path)| path == NAMESPACE)
            .cloned()
            .collect::<Vec<_>>(),
        [
            (Method::GET, NAMESPACE.to_owned()),
            (Method::PATCH, NAMESPACE.to_owned()),
        ]
    );
    assert_eq!(requests.len(), 3);
    assert!(requests.contains(&(Method::GET, WORKLOAD.to_owned())));
}
