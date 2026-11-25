use super::*;
use crate::kubernetes::MemoryResize;
use http_body_util::BodyExt;
use serde_json::{Value, json};

const RETAINED_REVISION: &str = "workspace.memeloop.dev/memory-resize-retained-revision";

#[tokio::test]
async fn live_memory_increase_only_patches_limit_and_keeps_ready_pod() {
    let (mut workspace, mut set, mut pod) = ready_pod_fixture();
    workspace.state = WorkspaceState::Ready;
    workspace.generation += 1;
    workspace.template.resources.memory_mib = 4_096;
    set["metadata"]["resourceVersion"] = json!("10");
    pod["metadata"]["resourceVersion"] = json!("11");
    pod["spec"] = set["spec"]["template"]["spec"].clone();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let builder =
        workspace_coordinator(Arc::new(WorkspaceDeleteMock::new("public-a", workspace.id))).builder;
    let service = service_fn({
        let calls = calls.clone();
        move |request: Request<kube::client::Body>| {
            let calls = calls.clone();
            let mut pod = pod.clone();
            let set = set.clone();
            async move {
                let path = request.uri().path().to_owned();
                let dry_run = request
                    .uri()
                    .query()
                    .is_some_and(|query| query.contains("dryRun=All"));
                let is_pod = path.contains("/pods/");
                if request.method() == Method::GET {
                    return Ok::<_, Infallible>(json_response(
                        StatusCode::OK,
                        if is_pod { pod } else { set },
                    ));
                }
                assert_eq!(request.method(), Method::PATCH);
                let payload: Value = serde_json::from_slice(
                    &request.into_body().collect().await.unwrap().to_bytes(),
                )
                .unwrap();
                if !is_pod {
                    assert_eq!(payload["metadata"]["uid"], "current-sts-uid");
                    assert_eq!(payload["metadata"]["resourceVersion"], "10");
                    assert_eq!(
                        payload["metadata"]["annotations"][RETAINED_REVISION],
                        "current-revision"
                    );
                    assert_eq!(
                        payload["spec"]["updateStrategy"],
                        json!({"type":"OnDelete","rollingUpdate":null})
                    );
                    assert_eq!(
                        payload["spec"]["template"],
                        json!({"spec":{"containers":[{
                            "name":"workspace","resources":{"limits":{"memory":"4096Mi"}}
                        }]}})
                    );
                    calls.lock().unwrap().push("save replacement");
                    return Ok(json_response(StatusCode::OK, set));
                }
                assert!(path.ends_with("/resize"));
                assert_eq!(
                    payload,
                    json!({
                        "metadata":{"uid":"current-pod-uid","resourceVersion":"11"},
                        "spec":{"containers":[{"name":"workspace","resources":{"limits":{"memory":"4096Mi"}}}]},
                    })
                );
                if dry_run {
                    calls.lock().unwrap().push("dry-run");
                } else {
                    calls.lock().unwrap().push("resize current Pod");
                    pod["status"]["containerStatuses"] = json!([{
                        "name":"workspace","ready":true,"restartCount":0,"image":"test","imageID":"test",
                        "resources":{"limits":{"memory":"4096Mi"}}
                    }]);
                }
                Ok(json_response(StatusCode::OK, pod))
            }
        }
    });
    let coordinator = KubernetesCoordinator::new(kube::Client::new(service, "default"), builder);
    assert!(matches!(
        coordinator
            .increase_workspace_memory(&workspace)
            .await
            .unwrap(),
        MemoryResize::Applied
    ));
    assert_eq!(
        &*calls.lock().unwrap(),
        &["dry-run", "save replacement", "resize current Pod"]
    );
}

#[tokio::test]
async fn unsupported_resize_does_not_mutate_the_workload() {
    let (mut workspace, mut set, mut pod) = ready_pod_fixture();
    workspace.state = WorkspaceState::Ready;
    workspace.generation += 1;
    workspace.template.resources.memory_mib = 4_096;
    set["metadata"]["resourceVersion"] = json!("10");
    pod["metadata"]["resourceVersion"] = json!("11");
    pod["spec"] = set["spec"]["template"]["spec"].clone();
    let builder =
        workspace_coordinator(Arc::new(WorkspaceDeleteMock::new("public-a", workspace.id))).builder;
    let service = service_fn(move |request: Request<kube::client::Body>| {
        let is_pod = request.uri().path().contains("/pods/");
        let response = if request.method() == Method::GET {
            json_response(
                StatusCode::OK,
                if is_pod { pod.clone() } else { set.clone() },
            )
        } else {
            assert_eq!(request.method(), Method::PATCH);
            assert!(request.uri().path().ends_with("/resize"));
            assert!(request.uri().query().unwrap().contains("dryRun=All"));
            json_response(
                StatusCode::NOT_FOUND,
                json!({
                    "apiVersion":"v1","kind":"Status","status":"Failure","reason":"NotFound","code":404,
                    "message":"the server could not find the requested resource",
                }),
            )
        };
        async move { Ok::<_, Infallible>(response) }
    });
    let coordinator = KubernetesCoordinator::new(kube::Client::new(service, "default"), builder);
    assert!(matches!(
        coordinator
            .increase_workspace_memory(&workspace)
            .await
            .unwrap(),
        MemoryResize::Rejected("unsupported")
    ));
}

#[tokio::test]
async fn resize_revision_keeps_runtime_ready_without_satisfying_start_or_restart() {
    let (workspace, mut set, pod) = ready_pod_fixture();
    set["metadata"]["generation"] = json!(11);
    set["metadata"]["annotations"] = json!({(RETAINED_REVISION): "current-revision"});
    set["spec"]["updateStrategy"] = json!({"type":"OnDelete"});
    set["status"]["updateRevision"] = json!("resized-revision");
    let builder =
        workspace_coordinator(Arc::new(WorkspaceDeleteMock::new("public-a", workspace.id))).builder;
    let service = service_fn(move |request: Request<kube::client::Body>| {
        assert_eq!(request.method(), Method::GET);
        let response = json_response(
            StatusCode::OK,
            if request.uri().path().contains("/pods/") {
                pod.clone()
            } else {
                set.clone()
            },
        );
        async move { Ok::<_, Infallible>(response) }
    });
    let coordinator = KubernetesCoordinator::new(kube::Client::new(service, "default"), builder);
    assert!(
        coordinator
            .has_running_workspace_connection(&workspace)
            .await
            .unwrap()
    );
    assert!(
        !coordinator
            .has_observed_replicas(&workspace, 1)
            .await
            .unwrap()
    );
}
