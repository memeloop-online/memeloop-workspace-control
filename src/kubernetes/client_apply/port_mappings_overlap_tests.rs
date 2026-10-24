use super::*;

use http_body_util::BodyExt;
use serde_json::Value;
use tokio::{sync::Semaphore, task::JoinHandle};

use crate::kubernetes::TtydMtlsConfig;

struct OverlapMock {
    workspace_id: Uuid,
    desired_name: String,
    route_path: String,
    requests: Mutex<Vec<(Method, String)>>,
    writes: Mutex<Vec<String>>,
    route_started: Semaphore,
    release_route: Semaphore,
    route_finished: Semaphore,
    lists_started: Semaphore,
    release_lists: Semaphore,
    lists_finished: Semaphore,
    failed_list_returned: Semaphore,
    desired_started: Semaphore,
    release_desired: Semaphore,
    fail_route: bool,
    fail_inventory: bool,
    fail_desired: bool,
}

impl OverlapMock {
    fn object(&self, kind: &str, name: &str) -> Value {
        json!({
            "apiVersion": if kind == "Service" { "v1" } else { "networking.k8s.io/v1" },
            "kind":kind,
            "metadata":{
                "name":name, "namespace":NAMESPACE,
                "labels":{
                    "workspace.memeloop.dev/owner-installation":"mapping-test",
                    "workspace.memeloop.dev/workspace-id":self.workspace_id.to_string(),
                    "workspace.memeloop.dev/port-mapping-id":"inventory-fixture",
                },
            },
        })
    }

    async fn respond(&self, request: Request<kube::client::Body>) -> Response<Body> {
        let path = request.uri().path().to_owned();
        let method = request.method().clone();
        self.requests
            .lock()
            .unwrap()
            .push((method.clone(), path.clone()));

        if method == Method::GET && path == self.route_path {
            self.route_started.add_permits(1);
            self.release_route.acquire().await.unwrap().forget();
            self.route_finished.add_permits(1);
            return if self.fail_route {
                response(StatusCode::BAD_GATEWAY, failure(502))
            } else {
                response(StatusCode::NOT_FOUND, failure(404))
            };
        }
        if method == Method::GET
            && let Some((_, kind)) = COLLECTIONS
                .iter()
                .find(|(collection, _)| *collection == path)
        {
            let selector = url::form_urlencoded::parse(request.uri().query().unwrap().as_bytes())
                .find_map(|(key, value)| (key == "labelSelector").then_some(value.into_owned()))
                .unwrap();
            assert_eq!(
                selector,
                format!(
                    "{OWNER_INSTALLATION_LABEL}=mapping-test,{WORKSPACE_ID_LABEL}={},{}",
                    self.workspace_id,
                    resource_port_mappings::PORT_MAPPING_ID_LABEL,
                )
            );
            self.lists_started.add_permits(1);
            self.release_lists.acquire().await.unwrap().forget();
            if self.fail_inventory && *kind == "Ingress" {
                self.failed_list_returned.add_permits(1);
                return response(StatusCode::INTERNAL_SERVER_ERROR, failure(500));
            }
            let (stale, desired) = if *kind == "NetworkPolicy" {
                (
                    "stale-mapping-ingress".to_owned(),
                    format!("{}-ingress", self.desired_name),
                )
            } else {
                ("stale-mapping".to_owned(), self.desired_name.clone())
            };
            self.lists_finished.add_permits(1);
            return response(
                StatusCode::OK,
                json!({
                    "apiVersion":if *kind == "Service" { "v1" } else { "networking.k8s.io/v1" },
                    "kind":format!("{kind}List"), "metadata":{"resourceVersion":"1"},
                    "items":[self.object(kind, &stale), self.object(kind, &desired)],
                }),
            );
        }
        if method == Method::GET && path.ends_with("/envoyfilters") {
            return response(
                StatusCode::OK,
                json!({"apiVersion":"networking.istio.io/v1alpha3", "kind":"EnvoyFilterList",
                    "metadata":{"resourceVersion":"1"}, "items":[]}),
            );
        }
        if method == Method::GET {
            // Existing desired resources must survive an inventory captured before
            // their updates. Other per-object reads represent absent resources.
            for (collection, kind) in COLLECTIONS {
                let name = if kind == "NetworkPolicy" {
                    format!("{}-ingress", self.desired_name)
                } else {
                    self.desired_name.clone()
                };
                if path == format!("{collection}/{name}") {
                    return response(StatusCode::OK, self.object(kind, &name));
                }
            }
            assert!(path.contains("/pods/") || path.contains("/envoyfilters/"));
            return response(StatusCode::NOT_FOUND, failure(404));
        }
        if method == Method::DELETE {
            return response(
                StatusCode::OK,
                json!({"apiVersion":"v1", "kind":"Status", "status":"Success", "code":200}),
            );
        }
        assert_eq!(method, Method::PATCH);
        if path == format!("{}/{}", COLLECTIONS[0].0, self.desired_name) {
            self.desired_started.add_permits(1);
            self.release_desired.acquire().await.unwrap().forget();
            if self.fail_desired {
                return response(StatusCode::SERVICE_UNAVAILABLE, failure(503));
            }
        }
        let mut body: Value =
            serde_json::from_slice(&request.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        body["metadata"]["resourceVersion"] = json!("2");
        self.writes.lock().unwrap().push(path);
        response(StatusCode::OK, body)
    }

    fn deletes(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(method, _)| *method == Method::DELETE)
            .map(|(_, path)| path.clone())
            .collect()
    }

    fn desired_paths(&self) -> Vec<String> {
        COLLECTIONS
            .iter()
            .map(|(collection, kind)| {
                let suffix = if *kind == "NetworkPolicy" {
                    "-ingress"
                } else {
                    ""
                };
                format!("{collection}/{}{suffix}", self.desired_name)
            })
            .collect()
    }
}

fn response(status: StatusCode, body: Value) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn failure(code: u16) -> Value {
    json!({"apiVersion":"v1", "kind":"Status", "status":"Failure",
        "reason":"FixtureFailure", "message":"controlled fixture response", "code":code})
}

fn setup_overlap(
    fail_route: bool,
    fail_inventory: bool,
    fail_desired: bool,
) -> (
    KubernetesCoordinator,
    Workspace,
    PortMapping,
    Arc<OverlapMock>,
) {
    let (coordinator, workspace, _) = setup(false, false);
    let mut builder = coordinator.builder;
    builder.port_mapping_domain = Some("apps.example".to_owned());
    builder
        .higress_pod_labels
        .insert("app".to_owned(), "higress-gateway".to_owned());
    builder.ttyd_mtls = Some(
        TtydMtlsConfig::new(
            "server".to_owned(),
            "ca".to_owned(),
            "higress-system".to_owned(),
            "client".to_owned(),
        )
        .unwrap(),
    );
    let mapping = PortMapping {
        id: Uuid::now_v7(),
        organization_id: workspace.organization_id,
        workspace_id: workspace.id,
        internal_port: 3000,
        display_name: None,
        created_by: workspace.owner_id,
        created_at: 1,
    };
    let names = builder.runtime_names(&workspace).unwrap();
    let mock = Arc::new(OverlapMock {
        workspace_id: workspace.id,
        desired_name: resource_port_mappings::name(&mapping),
        route_path: format!(
            "/api/v1/namespaces/{NAMESPACE}/configmaps/{}",
            names.resources.http_proxy_config
        ),
        requests: Mutex::new(Vec::new()),
        writes: Mutex::new(Vec::new()),
        route_started: Semaphore::new(0),
        release_route: Semaphore::new(0),
        route_finished: Semaphore::new(0),
        lists_started: Semaphore::new(0),
        release_lists: Semaphore::new(0),
        lists_finished: Semaphore::new(0),
        failed_list_returned: Semaphore::new(0),
        desired_started: Semaphore::new(0),
        release_desired: Semaphore::new(0),
        fail_route,
        fail_inventory,
        fail_desired,
    });
    let responder = mock.clone();
    let service = service_fn(move |request| {
        let responder = responder.clone();
        async move { Ok::<_, Infallible>(responder.respond(request).await) }
    });
    (
        KubernetesCoordinator::new(kube::Client::new(service, NAMESPACE), builder),
        workspace,
        mapping,
        mock,
    )
}

async fn arrived(signal: &Semaphore, count: u32) {
    tokio::time::timeout(Duration::from_secs(5), signal.acquire_many(count))
        .await
        .expect("the selected independent request must arrive")
        .unwrap()
        .forget();
}

type ReconcileTask = JoinHandle<Result<(), ReconcileError>>;

fn start(
    coordinator: KubernetesCoordinator,
    workspace: Workspace,
    mapping: PortMapping,
) -> ReconcileTask {
    tokio::spawn(async move {
        coordinator
            .reconcile_port_mappings(&workspace, &[mapping])
            .await
    })
}

async fn result(task: ReconcileTask) -> Result<(), ReconcileError> {
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("released reconciliation must finish")
        .unwrap()
}

async fn remains_pending(task: &mut ReconcileTask) {
    assert!(
        tokio::time::timeout(Duration::from_millis(50), task)
            .await
            .is_err(),
        "reconciliation must wait for both branches"
    );
}

#[tokio::test]
async fn inventory_overlaps_routes_and_preserves_desired_objects_before_pruning() {
    let (coordinator, workspace, mapping, mock) = setup_overlap(false, false, false);
    let task = start(coordinator, workspace, mapping);
    arrived(&mock.route_started, 1).await;
    arrived(&mock.lists_started, 3).await;
    mock.release_lists.add_permits(3);
    arrived(&mock.lists_finished, 3).await;
    assert!(mock.deletes().is_empty());
    assert!(mock.writes.lock().unwrap().is_empty());

    mock.release_route.add_permits(1);
    arrived(&mock.desired_started, 1).await;
    assert!(mock.deletes().is_empty());
    mock.release_desired.add_permits(1);
    result(task).await.unwrap();

    let writes = mock.writes.lock().unwrap();
    assert_eq!(writes.first(), Some(&mock.route_path));
    for path in mock.desired_paths() {
        assert!(writes.contains(&path));
    }
    assert_eq!(
        mock.deletes(),
        vec![
            format!("{}/stale-mapping", COLLECTIONS[0].0),
            format!("{}/stale-mapping", COLLECTIONS[1].0),
            format!("{}/stale-mapping-ingress", COLLECTIONS[2].0),
        ]
    );
}

#[tokio::test]
async fn failed_inventory_does_not_cancel_routes_or_desired_writes() {
    let (coordinator, workspace, mapping, mock) = setup_overlap(false, true, false);
    let mut task = start(coordinator, workspace, mapping);
    arrived(&mock.route_started, 1).await;
    arrived(&mock.lists_started, 3).await;
    mock.release_lists.add_permits(3);
    arrived(&mock.failed_list_returned, 1).await;
    remains_pending(&mut task).await;
    mock.release_route.add_permits(1);
    arrived(&mock.desired_started, 1).await;
    mock.release_desired.add_permits(1);
    assert!(matches!(
        result(task).await,
        Err(ReconcileError::Kubernetes(kube::Error::Api(status))) if status.code == 500,
    ));
    let writes = mock.writes.lock().unwrap();
    assert_eq!(writes.first(), Some(&mock.route_path));
    for path in mock.desired_paths() {
        assert!(writes.contains(&path));
    }
    assert!(mock.deletes().is_empty());
}

#[tokio::test]
async fn failed_routes_still_wait_for_inventory_and_keep_apply_error_priority() {
    let (coordinator, workspace, mapping, mock) = setup_overlap(true, true, false);
    let mut task = start(coordinator, workspace, mapping);
    arrived(&mock.route_started, 1).await;
    arrived(&mock.lists_started, 3).await;
    mock.release_route.add_permits(1);
    arrived(&mock.route_finished, 1).await;
    remains_pending(&mut task).await;
    mock.release_lists.add_permits(3);
    assert!(matches!(
        result(task).await,
        Err(ReconcileError::Kubernetes(kube::Error::Api(status))) if status.code == 502,
    ));
    assert!(mock.writes.lock().unwrap().is_empty());
    assert!(mock.deletes().is_empty());
}

#[tokio::test]
async fn failed_desired_write_does_not_prune_a_successful_inventory() {
    let (coordinator, workspace, mapping, mock) = setup_overlap(false, false, true);
    let task = start(coordinator, workspace, mapping);
    arrived(&mock.route_started, 1).await;
    arrived(&mock.lists_started, 3).await;
    mock.release_lists.add_permits(3);
    arrived(&mock.lists_finished, 3).await;
    mock.release_route.add_permits(1);
    arrived(&mock.desired_started, 1).await;
    mock.release_desired.add_permits(1);
    assert!(matches!(
        result(task).await,
        Err(ReconcileError::Kubernetes(kube::Error::Api(status))) if status.code == 503,
    ));
    assert_eq!(mock.writes.lock().unwrap().first(), Some(&mock.route_path));
    assert!(mock.deletes().is_empty());
}
