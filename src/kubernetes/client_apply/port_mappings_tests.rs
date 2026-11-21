use std::{
    collections::BTreeMap,
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
};

use http_body_util::BodyExt;

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

#[path = "port_mappings_overlap_tests.rs"]
mod overlap;

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

#[tokio::test]
async fn metadata_uses_ingress_hostname_and_reflects_edits_and_removal_without_credentials() {
    let (workspace, mut mapping) = overlap::metadata_fixture();
    let items = port_mapping_metadata(&workspace, &[mapping.clone()], Some("apps.example"));
    assert_eq!(
        items[0]["https_url"],
        format!(
            "https://{}",
            resource_port_mappings::hostname(&mapping, "apps.example")
        )
    );
    assert_eq!(items[0]["status"], "configured");
    assert_eq!(items[0].as_object().unwrap().len(), 6);
    mapping.display_name = Some("中文 \"quoted\" \\path".to_owned());
    mapping.internal_port = 9000;
    let updated = port_mapping_metadata(&workspace, &[mapping.clone()], Some("apps.example"));
    assert_eq!(updated[0]["display_name"], mapping.display_name.unwrap());
    assert_eq!(updated[0]["internal_port"], 9000);
    assert_eq!(updated[0]["https_url"], items[0]["https_url"]);
    assert_eq!(
        port_mapping_metadata(&workspace, &[], Some("apps.example")),
        json!([])
    );
}

#[tokio::test]
async fn metadata_changes_preserve_config_and_only_refresh_existing_pod_once_per_contents() {
    let (base, _, _) = setup(false, false);
    let (workspace, mut mapping) = overlap::metadata_fixture();
    let mut builder = base.builder;
    builder.port_mapping_domain = Some("apps.example".to_owned());
    let labels = builder.labels(workspace.id);
    let objects = Arc::new(Mutex::new((
        json!({"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"fixture-config","labels":labels},"data":{"sshd_config":"keep sshd","mwc-workspace-bootstrap":"keep script"}}),
        json!({"apiVersion":"v1","kind":"Pod","metadata":{"uid":"original-pod-uid","resourceVersion":"1","labels":labels},"spec":{"containers":[{"name":"workspace","image":"keep-image"}]}}),
        Vec::<String>::new(),
    )));
    let captured = objects.clone();
    let service = service_fn(move |request: Request<kube::client::Body>| {
        let captured = captured.clone();
        async move {
            let path = request.uri().path().to_owned();
            let method = request.method().clone();
            let body = request.into_body().collect().await.unwrap().to_bytes();
            let mut state = captured.lock().unwrap();
            let is_config = path.contains("/configmaps/");
            assert!(is_config || path.contains("/pods/"));
            if method == Method::PATCH {
                let patch: serde_json::Value = serde_json::from_slice(&body).unwrap();
                if is_config {
                    assert_eq!(patch["data"].as_object().unwrap().len(), 1);
                    state.0["data"]["port-mappings.json"] =
                        patch["data"]["port-mappings.json"].clone();
                } else {
                    assert_eq!(patch["metadata"]["uid"], "original-pod-uid");
                    assert!(patch.get("spec").is_none());
                    state.1["metadata"]["annotations"] = patch["metadata"]["annotations"].clone();
                }
                state.2.push(path);
            } else {
                assert_eq!(method, Method::GET);
            }
            let value = if is_config {
                state.0.clone()
            } else {
                state.1.clone()
            };
            Ok::<_, Infallible>(
                Response::builder()
                    .status(200)
                    .header("content-type", "application/json")
                    .body(Body::from(value.to_string()))
                    .unwrap(),
            )
        }
    });
    let coordinator = KubernetesCoordinator::new(kube::Client::new(service, NAMESPACE), builder);
    coordinator
        .apply_port_mapping_metadata(&workspace, &[mapping.clone()])
        .await
        .unwrap();
    coordinator
        .apply_port_mapping_metadata(&workspace, &[mapping.clone()])
        .await
        .unwrap();
    assert_eq!(
        objects.lock().unwrap().2.len(),
        2,
        "unchanged metadata never patches either object again"
    );
    mapping.display_name = Some("updated name".to_owned());
    mapping.internal_port = 9000;
    coordinator
        .apply_port_mapping_metadata(&workspace, &[mapping])
        .await
        .unwrap();
    coordinator
        .apply_port_mapping_metadata(&workspace, &[])
        .await
        .unwrap();
    let state = objects.lock().unwrap();
    assert_eq!(state.2.len(), 6);
    assert_eq!(state.0["data"]["sshd_config"], "keep sshd");
    assert_eq!(state.0["data"]["mwc-workspace-bootstrap"], "keep script");
    assert_eq!(state.0["data"]["port-mappings.json"], "[]");
    assert_eq!(state.1["metadata"]["uid"], "original-pod-uid");
    assert_eq!(state.1["spec"]["containers"][0]["image"], "keep-image");
}
