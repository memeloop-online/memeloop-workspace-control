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
    match tokio::try_join!(
        state.database.job_counts(),
        state.database.workspace_metrics()
    ) {
        Ok((jobs, workspace_metrics)) => {
            let mut body = format!(
                "# TYPE mwc_http_requests_total counter\nmwc_http_requests_total {}\n# TYPE mwc_jobs gauge\nmwc_jobs{{status=\"pending\"}} {}\nmwc_jobs{{status=\"running\"}} {}\nmwc_jobs{{status=\"completed\"}} {}\n# TYPE mwc_jobs_pending gauge\nmwc_jobs_pending {}\n# TYPE mwc_configured_replicas gauge\nmwc_configured_replicas {}\n",
                state.request_count(),
                jobs.pending,
                jobs.running,
                jobs.completed,
                jobs.pending,
                state.config.replica_count,
            );
            append_workspace_metrics(&mut body, &workspace_metrics);
            ([(CONTENT_TYPE, "text/plain; version=0.0.4")], body).into_response()
        }
        Err(error) => {
            tracing::error!(error = %error, "metrics database query failed");
            (StatusCode::SERVICE_UNAVAILABLE, "metrics unavailable").into_response()
        }
    }
}

fn append_workspace_metrics(body: &mut String, metrics: &crate::storage::WorkspaceMetrics) {
    use std::fmt::Write;

    body.push_str("# HELP mwc_workspaces Workspaces managed by lifecycle state.\n# TYPE mwc_workspaces gauge\n");
    for (state, count) in &metrics.states {
        let _ = writeln!(body, "mwc_workspaces{{state=\"{}\"}} {count}", label(state));
    }
    body.push_str("# HELP mwc_resource_requested Requested resources across all non-deleted workspaces.\n# TYPE mwc_resource_requested gauge\n");
    let total = metrics
        .users
        .iter()
        .fold(crate::quota::Resources::default(), |mut total, user| {
            total.cpu_millis = total.cpu_millis.saturating_add(user.resources.cpu_millis);
            total.memory_mib = total.memory_mib.saturating_add(user.resources.memory_mib);
            total.gpu_count = total.gpu_count.saturating_add(user.resources.gpu_count);
            total.disk_gib = total.disk_gib.saturating_add(user.resources.disk_gib);
            total
        });
    append_resources(body, "mwc_resource_requested", "", &total);
    body.push_str("# HELP mwc_user_workspaces Workspaces per owner and lifecycle state.\n# TYPE mwc_user_workspaces gauge\n");
    body.push_str("# HELP mwc_user_resource_requested Requested resources per workspace owner.\n# TYPE mwc_user_resource_requested gauge\n");
    for user in &metrics.users {
        let labels = format!("user_id=\"{}\"", user.user_id);
        for (state, count) in &user.states {
            let _ = writeln!(
                body,
                "mwc_user_workspaces{{{labels},state=\"{}\"}} {count}",
                label(state)
            );
        }
        append_resources(
            body,
            "mwc_user_resource_requested",
            &labels,
            &user.resources,
        );
    }
}

fn append_resources(
    body: &mut String,
    metric: &str,
    prefix: &str,
    resources: &crate::quota::Resources,
) {
    use std::fmt::Write;

    let separator = if prefix.is_empty() { "" } else { "," };
    let _ = writeln!(
        body,
        "{metric}{{{prefix}{separator}resource=\"cpu\",unit=\"millicores\"}} {}",
        resources.cpu_millis
    );
    let _ = writeln!(
        body,
        "{metric}{{{prefix}{separator}resource=\"memory\",unit=\"mebibytes\"}} {}",
        resources.memory_mib
    );
    let _ = writeln!(
        body,
        "{metric}{{{prefix}{separator}resource=\"gpu\",unit=\"devices\"}} {}",
        resources.gpu_count
    );
    let _ = writeln!(
        body,
        "{metric}{{{prefix}{separator}resource=\"disk\",unit=\"gibibytes\"}} {}",
        resources.disk_gib
    );
}

fn label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}
