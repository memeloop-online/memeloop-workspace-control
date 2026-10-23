use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use axum::http::{Method, Request, Response};

use kube::{
    Client, Config,
    client::{Body, ClientBuilder},
};
use tower::Service;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const READ_TIMEOUT: Duration = Duration::from_secs(15);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// Build the client for finite control-plane requests. Long-lived Kubernetes
/// watch/exec/attach streams must use a separately configured client.
pub fn request_client(mut config: Config) -> Result<Client, kube::Error> {
    config.connect_timeout = Some(CONNECT_TIMEOUT);
    config.read_timeout = Some(READ_TIMEOUT);
    config.write_timeout = Some(WRITE_TIMEOUT);
    // Persisted jobs already schedule retries. Hidden SDK retries, especially
    // Retry-After responses, would keep an executor occupied and unobservable.
    config.default_retry = false;
    let client = ClientBuilder::try_from(config)?
        .with_layer(&tower::layer::layer_fn(|inner| ApprovalHeaders { inner }))
        .build();
    tracing::info!(
        connect_timeout_seconds = CONNECT_TIMEOUT.as_secs(),
        read_timeout_seconds = READ_TIMEOUT.as_secs(),
        write_timeout_seconds = WRITE_TIMEOUT.as_secs(),
        sdk_retries = false,
        "configured finite Kubernetes API requests"
    );
    Ok(client)
}

/// Watches have a server-side 60-second lifetime. Keep their transport separate
/// so finite reconciliation calls still release their executor after 15 seconds.
pub fn watch_client(mut config: Config) -> Result<Client, kube::Error> {
    config.connect_timeout = Some(CONNECT_TIMEOUT);
    config.read_timeout = Some(Duration::from_secs(75));
    config.write_timeout = Some(WRITE_TIMEOUT);
    config.default_retry = false;
    Client::try_from(config)
}

// This layer observes call-to-headers only, inside the SDK's existing buffer.
// It never polls the response body; cancelled futures produce no completion event.
struct ApprovalHeaders<S> {
    inner: S,
}

impl<S, B> Service<Request<Body>> for ApprovalHeaders<S>
where
    S: Service<Request<Body>, Response = Response<B>>,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
    B: Send + 'static,
{
    type Response = Response<B>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<Body>) -> Self::Future {
        let path = request.uri().path();
        let targeted = request.method() == Method::GET
            && matches!(path.split('/').collect::<Vec<_>>().as_slice(),
                ["", "api", "v1", "namespaces", namespace, "configmaps", name]
                if !namespace.is_empty() && name.ends_with("-startup-gate"));
        let resource_path = targeted.then(|| path.to_owned());
        let started = Instant::now();
        let future = self.inner.call(request);
        Box::pin(async move {
            let result = future.await;
            if let Some(resource_path) = resource_path {
                let headers_ms = started.elapsed().as_secs_f64() * 1000.0;
                match &result {
                    Ok(response) => {
                        let audit_id = response
                            .headers()
                            .get("audit-id")
                            .and_then(|value| value.to_str().ok())
                            .and_then(|value| uuid::Uuid::parse_str(value).ok())
                            .map(|value| value.to_string());
                        tracing::info!(target: "mwc::kubernetes::approval_headers",
                            method = "GET", resource_path, headers_ms,
                            status = response.status().as_u16(), audit_id,
                            result = "headers", "startup approval transport completed");
                    }
                    Err(_) => tracing::info!(target: "mwc::kubernetes::approval_headers",
                        method = "GET", resource_path, headers_ms,
                        result = "service_error", "startup approval transport completed"),
                }
            }
            result
        })
    }
}

#[cfg(test)]
#[path = "request_client_tests.rs"]
mod tests;
