use std::{sync::Arc, time::Instant};

use axum::{
    extract::{MatchedPath, Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use tracing::Instrument;
use uuid::Uuid;

use super::AppState;

mod render;

#[cfg(test)]
#[path = "metrics_tests.rs"]
mod tests;

const OPENMETRICS_CONTENT_TYPE: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";

pub(super) async fn count(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let request_id = Uuid::now_v7();
    let method = match request.method().as_str() {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "DELETE" => "DELETE",
        "PATCH" => "PATCH",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        _ => "OTHER",
    };
    let request_bytes = content_length(request.headers());
    let route = route_label(&request);
    let span = tracing::info_span!("http_request", %request_id, method, route = %route);
    let observation = state.observability.begin_http(method, route, request_bytes);
    async move {
        let started = Instant::now();
        let mut response = next.run(request).await;
        response.headers_mut().insert(
            "x-request-id",
            request_id
                .to_string()
                .parse()
                .expect("UUID is a valid header"),
        );
        let elapsed = started.elapsed();
        let status = response.status().as_u16();
        observation.finish(status, content_length(response.headers()));
        let elapsed_ms = elapsed.as_secs_f64() * 1_000.0;
        if elapsed.as_secs() >= 1 || response.status().is_server_error() {
            tracing::info!(status, elapsed_ms, "HTTP request completed");
        } else {
            tracing::debug!(status, elapsed_ms, "HTTP request completed");
        }
        response
    }
    .instrument(span)
    .await
}

pub(super) async fn prometheus(State(state): State<Arc<AppState>>) -> Response {
    match tokio::try_join!(
        state.database.job_counts(),
        state.database.workspace_metrics()
    ) {
        Ok((jobs, workspace_metrics)) => {
            let runtime = state.observability.snapshot();
            let plugins = state.plugins.runtime_metrics();
            let mut body = String::with_capacity(16 * 1024);
            render::append_metrics(
                &mut body,
                render::MetricInput {
                    runtime: &runtime,
                    plugins,
                    kubernetes: state.kubernetes_client.is_some(),
                    prometheus: state.config.prometheus_url.is_some(),
                    webhook: state.cipher.is_some(),
                    jobs: &jobs,
                    replicas: state.config.replica_count,
                    workspace_metrics: &workspace_metrics,
                },
            );
            ([(header::CONTENT_TYPE, OPENMETRICS_CONTENT_TYPE)], body).into_response()
        }
        Err(error) => {
            tracing::error!(error = %error, "metrics database query failed");
            (StatusCode::SERVICE_UNAVAILABLE, "metrics unavailable").into_response()
        }
    }
}

fn route_label(request: &Request) -> String {
    request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_owned())
        .unwrap_or_else(|| {
            if request.uri().path().starts_with("/api/") {
                "unmatched_api".to_owned()
            } else {
                "ui_asset".to_owned()
            }
        })
}

fn content_length(headers: &axum::http::HeaderMap) -> u64 {
    headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
        .min(82 * 1024 * 1024)
}
