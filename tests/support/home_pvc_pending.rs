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
    crypto::EnvelopeCipher,
    jobs::{JobWorker, WorkspaceReconcileHandler},
    kubernetes::{InternetEgressConfig, KubernetesCoordinator, ResourceBuilder},
    storage::{Database, NewJob},
    workspaces::WorkspaceState,
};
use serde_json::{Value, json};
use tower::service_fn;

use super::support::{ADMIN, Fixture, NAMESPACE, put};

#[tokio::test]
async fn guarded_pending_job_adopts_current_home_and_reloads_binding_without_fallback() {
    let mut fixture = Fixture::new().await;
    fixture.ready().await;
    let (pod, sts) = fixture.adopted_runtime();
    let patches = Arc::new(Mutex::new(Vec::new()));
    let coordinator = coordinator(&fixture, sts.clone(), patches.clone());
    let builder = builder();
    let cipher =
        EnvelopeCipher::from_base64(&EnvelopeCipher::generate_base64_key().unwrap()).unwrap();
    let handler = Arc::new(WorkspaceReconcileHandler::new(
        fixture.database.clone(),
        cipher,
        builder,
        coordinator,
    ));
    let worker = JobWorker::new(fixture.database.clone(), handler, "guard-worker".to_owned());
    let job_id = fixture
        .database
        .enqueue_job(
            NewJob {
                kind: "reconcile_workspace".to_owned(),
                workspace_id: Some(fixture.workspace.id),
                payload: json!({"generation":fixture.workspace.generation}),
                available_at: fixture.now,
            },
            fixture.now,
        )
        .await
        .unwrap();
    for attempt in 0..12 {
        assert!(worker.run_once(fixture.now + attempt * 5).await.unwrap());
    }
    let Database::Sqlite { pool, .. } = &fixture.database else {
        unreachable!()
    };
    let (status, attempts): (String, i64) =
        sqlx::query_as("SELECT status, attempts FROM jobs WHERE id = ?1")
            .bind(job_id.to_string())
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!((status.as_str(), attempts), ("pending", 0));
    assert!(patches.lock().unwrap().is_empty());
    assert_eq!(
        fixture
            .database
            .get_workspace(fixture.workspace.id)
            .await
            .unwrap()
            .state,
        WorkspaceState::Ready
    );
    let (status, response) = put(
        fixture.app(fixture.claim(), vec![pod], vec![sts]),
        &fixture.workspace,
        ADMIN,
        fixture.input(),
        "pending-adoption",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let stored = fixture
        .database
        .get_workspace(fixture.workspace.id)
        .await
        .unwrap();
    assert_eq!(stored.state, WorkspaceState::Ready);
    assert_eq!(stored.generation, fixture.workspace.generation + 1);
    assert_eq!(stored.home_volume_binding, Some(fixture.binding()));
    assert!(worker.run_once(fixture.now + 60).await.unwrap());
    let (status, attempts, payload): (String, i64, String) =
        sqlx::query_as("SELECT status, attempts, payload_json FROM jobs WHERE id = ?1")
            .bind(job_id.to_string())
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!((status.as_str(), attempts), ("completed", 1));
    assert_eq!(
        serde_json::from_str::<Value>(&payload).unwrap()["generation"],
        fixture.workspace.generation
    );
    let patches = patches.lock().unwrap();
    assert_eq!(patches.len(), 1);
    let spec = &patches[0]["spec"];
    assert!(spec["volumeClaimTemplates"].is_null() || spec["volumeClaimTemplates"] == json!([]));
    let home = spec["template"]["spec"]["volumes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|volume| volume["name"] == "workspace-data")
        .unwrap();
    assert_eq!(
        home["persistentVolumeClaim"]["claimName"],
        fixture.binding().claim_name
    );
}

fn coordinator(
    fixture: &Fixture,
    sts: Value,
    patches: Arc<Mutex<Vec<Value>>>,
) -> KubernetesCoordinator {
    let claim = fixture.claim();
    let service = service_fn(move |request: Request<kube::client::Body>| {
        let sts = sts.clone();
        let claim = claim.clone();
        let patches = patches.clone();
        async move {
            let method = request.method().clone();
            let path = request.uri().path().to_owned();
            let (status, body) = if method == Method::PATCH {
                let bytes = request.into_body().collect().await.unwrap().to_bytes();
                let body: Value = serde_json::from_slice(&bytes).unwrap();
                if path.contains("/statefulsets/") {
                    patches.lock().unwrap().push(body.clone());
                }
                (StatusCode::OK, body)
            } else {
                assert_eq!(
                    method,
                    Method::GET,
                    "no deletion or volume replacement: {path}"
                );
                if path.contains("/statefulsets/") {
                    (StatusCode::OK, sts)
                } else if path.ends_with("/persistentvolumeclaims/migrated-home-20gi") {
                    (StatusCode::OK, claim)
                } else if let Some(kind) = [
                    ("/services", "ServiceList"),
                    ("/ingresses", "IngressList"),
                    ("/networkpolicies", "NetworkPolicyList"),
                ]
                .iter()
                .find_map(|(suffix, kind)| path.ends_with(suffix).then_some(*kind))
                {
                    (
                        StatusCode::OK,
                        json!({"apiVersion":"v1","kind":kind,"metadata":{},"items":[]}),
                    )
                } else {
                    (
                        StatusCode::NOT_FOUND,
                        json!({"apiVersion":"v1","kind":"Status","status":"Failure","reason":"NotFound","message":"fixture missing","code":404}),
                    )
                }
            };
            Ok::<_, Infallible>(
                Response::builder()
                    .status(status)
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
        }
    });
    KubernetesCoordinator::new(kube::Client::new(service, NAMESPACE), builder())
}

fn builder() -> ResourceBuilder {
    ResourceBuilder {
        installation_id: "home-binding".parse().unwrap(),
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
