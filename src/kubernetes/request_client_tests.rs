use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{Json, Router, http::StatusCode, response::IntoResponse};
use k8s_openapi::api::core::v1::ConfigMap;
use kube::{Api, Config};
use tokio::{net::TcpListener, sync::Notify, task::JoinHandle};

use super::request_client;

struct Server {
    api: Api<ConfigMap>,
    task: JoinHandle<()>,
}

impl Server {
    async fn start(router: Router) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let config = Config::new(format!("http://{address}").parse().unwrap());
        let api = Api::namespaced(request_client(config).unwrap(), "default");
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self { api, task }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn status_response(status: StatusCode) -> axum::response::Response {
    (
        status,
        [("retry-after", "120")],
        Json(serde_json::json!({
            "apiVersion": "v1", "kind": "Status", "status": "Failure",
            "message": "injected API failure", "reason": "ServiceUnavailable",
            "code": status.as_u16(),
        })),
    )
        .into_response()
}

#[tokio::test]
async fn retry_after_errors_surface_once_without_becoming_missing_objects() {
    for status in [
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::SERVICE_UNAVAILABLE,
        StatusCode::GATEWAY_TIMEOUT,
    ] {
        let attempts = Arc::new(AtomicUsize::new(0));
        let counter = attempts.clone();
        let server = Server::start(Router::new().fallback(move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                status_response(status)
            }
        }))
        .await;
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            server.api.get_metadata_opt("startup-approval"),
        )
        .await
        .expect("Retry-After must be handled by the job queue, not hidden SDK retries");
        assert!(matches!(result, Err(kube::Error::Api(error)) if error.code == status.as_u16()));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn unresponsive_api_read_times_out_instead_of_becoming_an_absent_object() {
    let entered = Arc::new(Notify::new());
    let signal = entered.clone();
    let server = Server::start(Router::new().fallback(move || {
        let signal = signal.clone();
        async move {
            signal.notify_one();
            std::future::pending::<StatusCode>().await
        }
    }))
    .await;
    let request = server.api.get_metadata_opt("startup-approval");
    tokio::pin!(request);
    tokio::select! {
        result = &mut request => panic!("request finished before the server accepted it: {result:?}"),
        () = entered.notified() => {},
        () = tokio::time::sleep(Duration::from_secs(3)) => panic!("mock server did not receive the request"),
    }
    let result = tokio::time::timeout(Duration::from_secs(20), request)
        .await
        .expect("a stalled response must release its executor within the read timeout");
    assert!(
        result.is_err(),
        "timeouts must never authorize creating over an unknown object"
    );
}

#[tokio::test]
async fn only_not_found_is_absence_and_successful_metadata_still_preserves_ownership() {
    let server = Server::start(Router::new().fallback(|uri: axum::http::Uri| async move {
        if uri.path().ends_with("/missing") {
            return status_response(StatusCode::NOT_FOUND);
        }
        Json(serde_json::json!({
            "apiVersion": "meta.k8s.io/v1", "kind": "PartialObjectMetadata",
            "metadata": {"name": "owned", "labels": {"workspace.memeloop.dev/owner-installation": "test-installation"}},
        }))
        .into_response()
    }))
    .await;
    assert!(
        server
            .api
            .get_metadata_opt("missing")
            .await
            .unwrap()
            .is_none()
    );
    let metadata = server.api.get_metadata_opt("owned").await.unwrap().unwrap();
    assert_eq!(metadata.metadata.name.as_deref(), Some("owned"));
    assert_eq!(
        metadata.metadata.labels.unwrap()["workspace.memeloop.dev/owner-installation"],
        "test-installation"
    );
}
