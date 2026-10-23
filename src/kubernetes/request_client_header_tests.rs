use super::*;
use crate::kubernetes::request_client::ApprovalHeaders;
use axum::{
    body::{Body, Bytes},
    http::{Request, Response, Uri},
};
use serde_json::{Value, json};
use std::{
    io::{self, Write},
    sync::Mutex,
};
use tower::{ServiceExt, service_fn};

const TARGET: &str = "mwc::kubernetes::approval_headers";
const AUDIT: &str = "019a1234-1234-7000-8000-123456789abc";
const PATH: &str = "/api/v1/namespaces/default/configmaps/probe-startup-gate";

#[derive(Clone, Default)]
struct Capture {
    bytes: Arc<Mutex<Vec<u8>>>,
    changed: Arc<Notify>,
}
impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes.lock().unwrap().extend_from_slice(bytes);
        self.changed.notify_one();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Capture {
    fn events(&self) -> Vec<Value> {
        String::from_utf8(self.bytes.lock().unwrap().clone())
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|event| event["target"] == TARGET)
            .collect()
    }
    async fn wait_for(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let changed = self.changed.notified();
                if self.events().len() >= count {
                    break;
                }
                changed.await;
            }
        })
        .await
        .expect("headers event must arrive independently of the body gate");
    }
}

fn capture_in_child(name: &str) -> Option<Capture> {
    let name = name
        .strip_prefix(concat!(env!("CARGO_CRATE_NAME"), "::"))
        .unwrap();
    const MARKER: &str = "MWC_APPROVAL_HEADER_CAPTURE_CHILD";
    if std::env::var(MARKER).as_deref() != Ok(name) {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture", "--color", "never"])
            .env(MARKER, name)
            .output()
            .expect("start isolated approval header test");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success()
                && stdout.contains(&format!("test {name} ... ok"))
                && stdout.contains("test result: ok. 1 passed; 0 failed; 0 ignored;"),
            "isolated approval header test failed: {stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return None;
    }
    let capture = Capture::default();
    let writer = capture.clone();
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(format!("off,{TARGET}=info"))
            .with_writer(move || writer.clone())
            .finish(),
    )
    .unwrap();
    Some(capture)
}

fn object() -> Value {
    json!({"apiVersion":"v1", "kind":"ConfigMap", "metadata":{
        "name":"probe-startup-gate", "labels":{"workspace.memeloop.dev/owner-installation":"owned"}},
        "immutable":true, "data":{"approved":"true", "sentinel":"response-body-secret"}})
}

async fn boundary(capture: &Capture, delay_body: bool) {
    let before = capture.events().len();
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let attempts = Arc::new(AtomicUsize::new(0));
    let (signal, gate, counter) = (entered.clone(), release.clone(), attempts.clone());
    let server = Server::start(Router::new().fallback(move || {
        let (signal, gate, counter) = (signal.clone(), gate.clone(), counter.clone());
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            signal.notify_one();
            if !delay_body {
                gate.notified().await;
            }
            let body = if delay_body {
                Body::from_stream(futures::stream::once(async move {
                    gate.notified().await;
                    Ok::<_, io::Error>(Bytes::from(object().to_string()))
                }))
            } else {
                Body::from(object().to_string())
            };
            Response::builder()
                .header("content-type", "application/json")
                .header("audit-id", AUDIT)
                .body(body)
                .unwrap()
        }
    }))
    .await;
    let api = server.api.clone();
    let task = tokio::spawn(async move { api.get_opt("probe-startup-gate").await });
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    if delay_body {
        capture.wait_for(before + 1).await;
        assert!(
            !task.is_finished(),
            "headers event must not wait for the SDK body collection"
        );
    } else {
        assert_eq!(
            capture.events().len(),
            before,
            "no headers event before server releases headers"
        );
        assert!(!task.is_finished());
    }
    release.notify_one();
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(serde_json::to_value(result).unwrap(), object());
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    let events = capture.events();
    assert_eq!(events.len(), before + 1);
    assert_eq!(events[before]["fields"]["resource_path"], PATH);
    assert_eq!(events[before]["fields"]["status"], 200);
    assert_eq!(events[before]["fields"]["audit_id"], AUDIT);
    assert!(events[before]["fields"]["headers_ms"].is_number());
}

#[tokio::test]
async fn approval_headers_distinguish_header_wait_from_body_wait() {
    let Some(capture) = capture_in_child(concat!(
        module_path!(),
        "::approval_headers_distinguish_header_wait_from_body_wait"
    )) else {
        return;
    };
    boundary(&capture, false).await;
    boundary(&capture, true).await;
    assert!(
        !String::from_utf8(capture.bytes.lock().unwrap().clone())
            .unwrap()
            .contains("response-body-secret")
    );
}

#[tokio::test]
async fn approval_header_statuses_preserve_sdk_absence_and_api_errors() {
    let Some(capture) = capture_in_child(concat!(
        module_path!(),
        "::approval_header_statuses_preserve_sdk_absence_and_api_errors"
    )) else {
        return;
    };
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = attempts.clone();
    let server = Server::start(Router::new().fallback(move |uri: Uri| {
        let counter = counter.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            let status = if uri.path().contains("missing") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            };
            let mut response = status_response(status);
            response
                .headers_mut()
                .insert("audit-id", "invalid-audit-secret".parse().unwrap());
            response
        }
    }))
    .await;
    assert!(
        server
            .api
            .get_opt("missing-startup-gate")
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(server.api.get_opt("busy-startup-gate").await,
        Err(kube::Error::Api(error)) if error.code == 503 && error.message == "injected API failure"));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    let events = capture.events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["fields"]["status"], 404);
    assert_eq!(events[1]["fields"]["status"], 503);
    assert!(
        events
            .iter()
            .all(|event| event["fields"].get("audit_id").is_none())
    );
    let output = String::from_utf8(capture.bytes.lock().unwrap().clone()).unwrap();
    assert!(!output.contains("invalid-audit-secret"));
    assert!(!output.contains("injected API failure"));
}

#[tokio::test]
async fn approval_headers_return_the_same_response_and_service_error_without_secret_logs() {
    let Some(capture) = capture_in_child(concat!(
        module_path!(),
        "::approval_headers_return_the_same_response_and_service_error_without_secret_logs"
    )) else {
        return;
    };
    let body = Arc::new("response-body-secret".to_owned());
    let original = body.clone();
    let counter = Arc::new(AtomicUsize::new(0));
    let calls = counter.clone();
    let inner = service_fn(move |request: Request<kube::client::Body>| {
        calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.uri().query(), Some("access_token=query-secret"));
        assert_eq!(request.headers()["authorization"], "Bearer header-secret");
        let body = body.clone();
        async move {
            use http_body_util::BodyExt;
            assert_eq!(
                request
                    .into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes()
                    .as_ref(),
                b"request-body-secret"
            );
            Ok::<_, io::Error>(
                Response::builder()
                    .status(201)
                    .header("audit-id", AUDIT)
                    .header("x-private", "response-header-secret")
                    .body(body)
                    .unwrap(),
            )
        }
    });
    let response = ApprovalHeaders { inner }
        .oneshot(
            Request::get(format!("{PATH}?access_token=query-secret"))
                .header("authorization", "Bearer header-secret")
                .body(kube::client::Body::from(b"request-body-secret".to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    assert_eq!(response.headers()["x-private"], "response-header-secret");
    assert!(Arc::ptr_eq(response.body(), &original));
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    let error = Arc::new(io::Error::other("service-error-secret"));
    let same = error.clone();
    let inner = service_fn(move |_: Request<kube::client::Body>| {
        let error = error.clone();
        async move { Err::<Response<()>, _>(error) }
    });
    let returned = ApprovalHeaders { inner }
        .oneshot(
            Request::get(PATH)
                .body(kube::client::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap_err();
    assert!(Arc::ptr_eq(&returned, &same));
    let events = capture.events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1]["fields"]["result"], "service_error");
    excluded_requests(&capture).await;
    let output = String::from_utf8(capture.bytes.lock().unwrap().clone()).unwrap();
    for sentinel in [
        "query-secret",
        "header-secret",
        "response-header-secret",
        "request-body-secret",
        "response-body-secret",
        "service-error-secret",
    ] {
        assert!(!output.contains(sentinel), "new target exposed {sentinel}");
    }
}

async fn excluded_requests(capture: &Capture) {
    let before = capture.events().len();
    for (method, path) in [
        ("POST", PATH),
        (
            "GET",
            "/api/v1/namespaces/default/secrets/probe-startup-gate",
        ),
        ("GET", "/api/v1/namespaces/default/configmaps/probe"),
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let inner = service_fn(move |_: Request<kube::client::Body>| {
            counter.fetch_add(1, Ordering::SeqCst);
            async { Ok::<_, io::Error>(Response::new(())) }
        });
        ApprovalHeaders { inner }
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .body(kube::client::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
    assert_eq!(capture.events().len(), before);
}
