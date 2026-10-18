use std::{error::Error, future::Future, io, time::Instant};

use uuid::Uuid;

use super::ReconcileError;

const MAX_ERROR_CHAIN: usize = 16;

/// Only static classifications and HTTP status codes may leave this boundary.
/// Error Display/Debug, API status messages and request/response bodies can contain
/// injected credentials, so never format the original error or any of its sources.
#[derive(Debug)]
pub(super) struct ErrorSummary {
    pub kind: &'static str,
    pub io_kind: &'static str,
    pub api_code: Option<u16>,
    pub source_kinds: Vec<&'static str>,
    pub truncated: bool,
}

impl ErrorSummary {
    pub fn new<E: Error + 'static>(error: &E) -> Self {
        let mut summary = Self {
            kind: "unknown",
            io_kind: "none",
            api_code: None,
            source_kinds: Vec::new(),
            truncated: false,
        };
        let mut current: Option<&(dyn Error + 'static)> = Some(error);
        while let Some(error) = current {
            if summary.source_kinds.len() == MAX_ERROR_CHAIN {
                summary.truncated = true;
                break;
            }
            // Transparent wrappers may forward source() past kube::Error itself.
            let kubernetes = error.downcast_ref::<kube::Error>().or_else(|| {
                match error.downcast_ref::<ReconcileError>() {
                    Some(ReconcileError::Kubernetes(error)) => Some(error),
                    _ => None,
                }
            });
            let category = if let Some(error) = kubernetes {
                let kind = match error {
                    kube::Error::Api(status) => {
                        summary.api_code = Some(status.code);
                        "api"
                    }
                    kube::Error::Service(_) => "service",
                    kube::Error::HyperError(_) => "hyper",
                    kube::Error::ReadEvents(_) => "read_events",
                    kube::Error::HttpError(_) => "http",
                    kube::Error::SerdeError(_) => "deserialize",
                    kube::Error::BuildRequest(_) => "build_request",
                    kube::Error::RustlsTls(_) => "tls",
                    kube::Error::Auth(_) => "auth",
                    _ => "other_kubernetes",
                };
                if summary.kind == "unknown" {
                    summary.kind = kind;
                }
                kind
            } else if let Some(error) = error.downcast_ref::<io::Error>() {
                summary.io_kind = match error.kind() {
                    io::ErrorKind::TimedOut => "timed_out",
                    io::ErrorKind::ConnectionReset => "connection_reset",
                    io::ErrorKind::ConnectionRefused => "connection_refused",
                    io::ErrorKind::ConnectionAborted => "connection_aborted",
                    io::ErrorKind::NotConnected => "not_connected",
                    io::ErrorKind::BrokenPipe => "broken_pipe",
                    io::ErrorKind::UnexpectedEof => "unexpected_eof",
                    _ => "other_io",
                };
                "io"
            } else if error.is::<tokio::time::error::Elapsed>() {
                "deadline_elapsed"
            } else {
                "unknown"
            };
            summary.source_kinds.push(category);
            current = error.source();
        }
        summary
    }
}

/// Keep fast operations in the existing aggregate phase logs. Emit one additional
/// record only when an individual request is slow or fails, identifying the exact
/// verb and object without exposing headers, query parameters or payload values.
pub(super) async fn observe_request<T>(
    workspace_id: Uuid,
    resource_kind: &'static str,
    object_name: &str,
    operation: &'static str,
    request: impl Future<Output = Result<T, kube::Error>>,
) -> Result<T, kube::Error> {
    let started = Instant::now();
    let result = request.await;
    let elapsed = started.elapsed();
    if result.is_err() || elapsed.as_secs() >= 1 {
        let error = result.as_ref().err().map(ErrorSummary::new);
        tracing::info!(
            %workspace_id,
            resource_kind,
            object_name,
            operation,
            elapsed_ms = elapsed.as_secs_f64() * 1000.0,
            success = result.is_ok(),
            error_kind = error.as_ref().map(|e| e.kind),
            error_io_kind = error.as_ref().map(|e| e.io_kind),
            error_api_code = error.as_ref().and_then(|e| e.api_code),
            error_chain = ?error.as_ref().map(|e| &e.source_kinds),
        error_chain_depth = error.as_ref().map(|e| e.source_kinds.len()),
            error_chain_truncated = error.as_ref().map(|e| e.truncated),
            "workspace Kubernetes request completed"
        );
    }
    result
}

#[cfg(test)]
#[path = "request_observation_tests.rs"]
mod tests;
