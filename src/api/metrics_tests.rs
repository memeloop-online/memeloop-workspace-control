use std::{
    io::{self, Write},
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::Request,
    http::{Method, StatusCode},
    middleware::{Next, from_fn, from_fn_with_state},
    routing::get,
};
use tokio::sync::Notify;
use tower::ServiceExt;
use tracing::instrument::WithSubscriber;
use uuid::Uuid;

use super::{AppState, count};
use crate::{config::AppConfig, storage::Database};

async fn state() -> Arc<AppState> {
    let installation_id: crate::config::InstallationId = "http-tracing-test".parse().unwrap();
    let database = Database::connect("sqlite::memory:", installation_id.clone())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    Arc::new(AppState::new(
        AppConfig {
            installation_id,
            listen_address: SocketAddr::from(([127, 0, 0, 1], 0)),
            database_url: "sqlite::memory:".to_owned(),
            replica_count: 1,
            instance_id: "test".to_owned(),
            ssh_public_host: None,
            internal_ssh_host: None,
            web_shell_public_origin: None,
            port_mapping_public_domain: None,
            prometheus_url: None,
            plugin_dir: None,
        },
        database,
    ))
}

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for LogBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// Other parallel tests use the same static callsites without a subscriber.
// Keep log-capture assertions in their own process so their dispatcher is isolated.
fn run_capture_in_child(name: &str) -> bool {
    const MARKER: &str = "MWC_HTTP_TRACE_CAPTURE_CHILD";
    if std::env::var(MARKER).as_deref() == Ok(name) {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture", "--color", "never"])
        .env(MARKER, name)
        .output()
        .expect("start isolated HTTP trace capture test");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success()
            && stdout.contains(&format!("test {name} ... ok"))
            && stdout.contains("test result: ok. 1 passed; 0 failed; 0 ignored;"),
        "isolated HTTP trace capture failed: {name}\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

#[tokio::test]
async fn public_requests_get_server_ids_and_bounded_safe_trace_fields() {
    if run_capture_in_child(
        "api::metrics::tests::public_requests_get_server_ids_and_bounded_safe_trace_fields",
    ) {
        return;
    }
    let state = state().await;
    let app = crate::api::router(state.clone());
    let buffer = LogBuffer::default();
    let writer = buffer.clone();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    let mut ids = Vec::new();
    async {
        for method in [Method::GET, Method::from_bytes(b"PRIVATE-METHOD").unwrap()] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri("/api/v1/workspaces/aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa?access_token=query-secret")
                        .header("x-request-id", "spoofed-client-id")
                        .header("authorization", "Bearer header-secret")
                        .body(Body::from("payload-secret"))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert!(response.status().is_client_error());
            let id = Uuid::parse_str(response.headers()["x-request-id"].to_str().unwrap())
                .unwrap();
            assert_eq!(id.get_version_num(), 7);
            ids.push(id);
        }
    }
    .with_subscriber(subscriber)
    .await;
    assert_ne!(ids[0], ids[1]);
    let output = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
    for id in ids {
        assert!(output.contains(&id.to_string()));
    }
    assert!(output.contains("/api/v1/workspaces/{workspace_id}"));
    assert!(output.contains("OTHER"));
    for secret in [
        "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
        "query-secret",
        "header-secret",
        "payload-secret",
        "spoofed-client-id",
        "PRIVATE-METHOD",
    ] {
        assert!(!output.contains(secret), "trace exposed {secret}");
    }
    let snapshot = state.observability.snapshot();
    assert_eq!(snapshot.active_http, 0);
    assert_eq!(snapshot.http.len(), 2);
    assert!(snapshot.http.iter().all(|series| {
        series.route == "/api/v1/workspaces/{workspace_id}"
            && matches!(series.method, "GET" | "OTHER")
    }));
}

#[tokio::test]
async fn server_errors_have_info_completion_even_when_fast() {
    if run_capture_in_child(
        "api::metrics::tests::server_errors_have_info_completion_even_when_fast",
    ) {
        return;
    }
    let state = state().await;
    let app = Router::new()
        .route(
            "/failure",
            get(|| async { StatusCode::SERVICE_UNAVAILABLE }),
        )
        .layer(from_fn_with_state(state.clone(), count));
    let buffer = LogBuffer::default();
    let writer = buffer.clone();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_max_level(tracing::Level::INFO)
        .with_writer(move || writer.clone())
        .finish();
    let response = app
        .oneshot(Request::get("/failure").body(Body::empty()).unwrap())
        .with_subscriber(subscriber)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let output = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
    let event: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
    assert_eq!(event["level"], "INFO");
    assert_eq!(event["fields"]["status"], 503);
    assert!(event["fields"]["elapsed_ms"].is_number());
    assert_eq!(
        event["span"]["request_id"],
        response.headers()["x-request-id"].to_str().unwrap()
    );
    assert_eq!(
        state.observability.snapshot().http[0].requests_by_status["5xx"],
        1
    );
}

fn gated_inner_middleware(
    state: Arc<AppState>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
) -> Router {
    let observed = state.clone();
    Router::new()
        .route("/api/v1/staged", get(|| async { StatusCode::NO_CONTENT }))
        .layer(from_fn(move |request: Request, next: Next| {
            let (state, entered, release) = (observed.clone(), entered.clone(), release.clone());
            async move {
                assert_eq!(state.observability.snapshot().active_http, 1);
                entered.notify_one();
                release.notified().await;
                next.run(request).await
            }
        }))
        .layer(from_fn_with_state(state, count))
}

#[tokio::test]
async fn inner_middleware_wait_is_counted_and_cancellation_preserves_aborted_guard() {
    let state = state().await;
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let app = gated_inner_middleware(state.clone(), entered.clone(), release.clone());
    let pending = tokio::spawn(
        app.clone()
            .oneshot(Request::get("/api/v1/staged").body(Body::empty()).unwrap()),
    );
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    assert_eq!(state.observability.snapshot().active_http, 1);
    release.notify_one();
    let response = pending.await.unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(response.headers().contains_key("x-request-id"));

    let cancelled =
        tokio::spawn(app.oneshot(Request::get("/api/v1/staged").body(Body::empty()).unwrap()));
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    assert_eq!(state.observability.snapshot().active_http, 1);
    cancelled.abort();
    assert!(cancelled.await.unwrap_err().is_cancelled());
    let snapshot = state.observability.snapshot();
    assert_eq!(snapshot.active_http, 0);
    let series = &snapshot.http[0];
    assert_eq!(series.route, "/api/v1/staged");
    assert_eq!(series.requests_by_status["2xx"], 1);
    assert_eq!(series.requests_by_status["aborted"], 1);
    assert!(series.duration_sum > 0.0);
}
