use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{StatusCode, header::CONTENT_TYPE},
    middleware::Next,
    response::{IntoResponse, Response},
};

use super::AppState;

pub(super) async fn count(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    state.count_request();
    next.run(request).await
}

pub(super) async fn prometheus(State(state): State<Arc<AppState>>) -> Response {
    match state.database.job_counts().await {
        Ok(jobs) => {
            let body = format!(
                "# TYPE mwc_http_requests_total counter\nmwc_http_requests_total {}\n# TYPE mwc_jobs gauge\nmwc_jobs{{status=\"pending\"}} {}\nmwc_jobs{{status=\"running\"}} {}\nmwc_jobs{{status=\"completed\"}} {}\n# TYPE mwc_jobs_pending gauge\nmwc_jobs_pending {}\n# TYPE mwc_configured_replicas gauge\nmwc_configured_replicas {}\n",
                state.request_count(),
                jobs.pending,
                jobs.running,
                jobs.completed,
                jobs.pending,
                state.config.replica_count,
            );
            ([(CONTENT_TYPE, "text/plain; version=0.0.4")], body).into_response()
        }
        Err(error) => {
            tracing::error!(error = %error, "metrics database query failed");
            (StatusCode::SERVICE_UNAVAILABLE, "metrics unavailable").into_response()
        }
    }
}
