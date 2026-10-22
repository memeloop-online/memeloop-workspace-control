use std::{
    collections::BTreeMap,
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    body::Body,
    http::{Method, Request, Response, StatusCode},
};
use serde_json::json;
use tokio::sync::Barrier;
use tower::service_fn;
use uuid::Uuid;

use super::*;
use crate::{
    kubernetes::ResourceBuilder,
    quota::Resources,
    templates::WorkspaceTemplateSpec,
    workspace_runtime::WorkspaceRuntimeIdentity,
    workspaces::{AccessMode, WorkspaceState},
};

const NAMESPACE: &str = "memeloop-workspace-control";
const COLLECTIONS: [(&str, &str); 3] = [
    (
        "/api/v1/namespaces/memeloop-workspace-control/services",
        "Service",
    ),
    (
        "/apis/networking.k8s.io/v1/namespaces/memeloop-workspace-control/ingresses",
        "Ingress",
    ),
    (
        "/apis/networking.k8s.io/v1/namespaces/memeloop-workspace-control/networkpolicies",
        "NetworkPolicy",
    ),
];

struct PruneMock {
    workspace_id: Uuid,
    lists_arrived: Barrier,
    requests: Mutex<Vec<(Method, String)>>,
    foreign_service: bool,
    failing_ingress_list: bool,
}

impl PruneMock {
    async fn respond(&self, request: Request<kube::client::Body>) -> Response<Body> {
        let path = request.uri().path().to_owned();
        let method = request.method().clone();
        self.requests
            .lock()
            .unwrap()
            .push((method.clone(), path.clone()));
        let (status, body) = if method == Method::GET {
            let (_, kind) = COLLECTIONS
                .iter()
                .find(|(collection, _)| *collection == path)
                .expect("only mapping cleanup LISTs are expected");
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
            // Three requests plus the test must arrive before any LIST can finish.
            // A serial implementation blocks on its first request and times out.
            self.lists_arrived.wait().await;
            if self.failing_ingress_list && *kind == "Ingress" {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({
                        "apiVersion":"v1", "kind":"Status", "status":"Failure",
                        "reason":"InternalError", "message":"fixture list failure", "code":500,
                    }),
                )
            } else {
                let name = if *kind == "NetworkPolicy" {
                    "stale-mapping-ingress"
                } else {
                    "stale-mapping"
                };
                let owner = if self.foreign_service && *kind == "Service" {
                    "foreign"
                } else {
                    "mapping-test"
                };
                (
                    StatusCode::OK,
                    json!({
                        "apiVersion": if *kind == "Service" { "v1" } else { "networking.k8s.io/v1" },
                        "kind":format!("{kind}List"), "metadata":{"resourceVersion":"1"},
                        "items":[{
                            "apiVersion":if *kind == "Service" { "v1" } else { "networking.k8s.io/v1" },
                            "kind":kind, "metadata":{
                                "name":name, "namespace":NAMESPACE,
                                "labels":{
                                    "workspace.memeloop.dev/owner-installation":owner,
                                    "workspace.memeloop.dev/workspace-id":self.workspace_id.to_string(),
                                    "workspace.memeloop.dev/port-mapping-id":"stale-mapping",
                                },
                            },
                        }],
                    }),
                )
            }
        } else {
            assert_eq!(method, Method::DELETE);
            assert!(
                COLLECTIONS
                    .iter()
                    .any(|(collection, _)| path.starts_with(&format!("{collection}/")))
            );
            (
                StatusCode::OK,
                json!({"apiVersion":"v1", "kind":"Status", "status":"Success", "code":200}),
            )
        };
        Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
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
}

fn setup(
    foreign_service: bool,
    failing_ingress_list: bool,
) -> (KubernetesCoordinator, Workspace, Arc<PruneMock>) {
    let workspace = Workspace {
        id: Uuid::now_v7(),
        short_id: "8000000000000001".to_owned(),
        organization_id: Uuid::now_v7(),
        owner_id: Uuid::now_v7(),
        name: "mapping cleanup".to_owned(),
        template_id: None,
        home_volume_binding: None,
        scratch_volume_binding: None,
        node_pool: "default".to_owned(),
        runtime: WorkspaceRuntimeIdentity,
        template: WorkspaceTemplateSpec::standard(
            "example/workspace:1",
            AccessMode::Internal,
            Resources {
                cpu_millis: 500,
                memory_mib: 512,
                gpu_count: 0,
                disk_gib: 1,
            },
        ),
        state: WorkspaceState::Provisioning,
        generation: 1,
        created_at: 1,
        updated_at: 1,
    };
    let builder = ResourceBuilder {
        installation_id: "mapping-test".parse().unwrap(),
        ttyd_image: "example/ttyd:1".to_owned(),
        buildkit_image: "example/buildkit:1".to_owned(),
        ttyd_mtls: None,
        higress_namespace: "higress-system".to_owned(),
        higress_pod_labels: BTreeMap::new(),
        higress_source_cidrs: Vec::new(),
        internet_egress: None,
        jump_host_namespace: "access".to_owned(),
        jump_host_pod_labels: BTreeMap::new(),
        storage_class_name: None,
        scratch_storage_class_name: "scratch".to_owned(),
        web_shell_domain: None,
        port_mapping_domain: None,
        higress_gateway_name: "higress".to_owned(),
        higress_https_section_name: "https".to_owned(),
        internal_ssh_node_port_enabled: false,
    };
    let mock = Arc::new(PruneMock {
        workspace_id: workspace.id,
        lists_arrived: Barrier::new(4),
        requests: Mutex::new(Vec::new()),
        foreign_service,
        failing_ingress_list,
    });
    let responder = mock.clone();
    let service = service_fn(move |request| {
        let responder = responder.clone();
        async move { Ok::<_, Infallible>(responder.respond(request).await) }
    });
    (
        KubernetesCoordinator::new(kube::Client::new(service, NAMESPACE), builder),
        workspace,
        mock,
    )
}

async fn reconcile(
    coordinator: &KubernetesCoordinator,
    workspace: &Workspace,
    mock: &PruneMock,
) -> Result<(), ReconcileError> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let (result, _) = tokio::join!(
            coordinator.reconcile_port_mappings(workspace, &[]),
            mock.lists_arrived.wait(),
        );
        result
    })
    .await
    .expect("all three cleanup LISTs must arrive before any response is released")
}

#[tokio::test]
async fn empty_mappings_list_concurrently_and_delete_owned_residue_in_order() {
    let (coordinator, workspace, mock) = setup(false, false);
    reconcile(&coordinator, &workspace, &mock).await.unwrap();
    assert_eq!(
        mock.deletes(),
        vec![
            format!("{}/stale-mapping", COLLECTIONS[0].0),
            format!("{}/stale-mapping", COLLECTIONS[1].0),
            format!("{}/stale-mapping-ingress", COLLECTIONS[2].0),
        ]
    );
    let requests = mock.requests.lock().unwrap();
    assert_eq!(requests.len(), 6);
    assert!(
        requests[..3]
            .iter()
            .all(|(method, _)| *method == Method::GET)
    );
}

#[tokio::test]
async fn empty_mappings_refuse_foreign_residue_after_concurrent_lists() {
    let (coordinator, workspace, mock) = setup(true, false);
    assert!(matches!(
        reconcile(&coordinator, &workspace, &mock).await,
        Err(ReconcileError::Ownership(_))
    ));
    assert!(mock.deletes().is_empty());
}

#[tokio::test]
async fn failed_cleanup_list_keeps_reconciliation_failed_without_deletes() {
    let (coordinator, workspace, mock) = setup(false, true);
    assert!(matches!(
        reconcile(&coordinator, &workspace, &mock).await,
        Err(ReconcileError::Kubernetes(kube::Error::Api(status))) if status.code == 500,
    ));
    assert!(mock.deletes().is_empty());
}
