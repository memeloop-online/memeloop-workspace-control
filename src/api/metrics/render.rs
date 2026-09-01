use std::{
    fmt::Write,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    observability::{HTTP_DURATION_BUCKETS, RuntimeSnapshot},
    plugins::PluginRuntimeMetrics,
};

pub(super) struct MetricInput<'a> {
    pub runtime: &'a RuntimeSnapshot,
    pub plugins: PluginRuntimeMetrics,
    pub kubernetes: bool,
    pub prometheus: bool,
    pub webhook: bool,
    pub jobs: &'a crate::storage::JobCounts,
    pub replicas: u16,
    pub workspace_metrics: &'a crate::storage::WorkspaceMetrics,
}

pub(super) fn append_metrics(body: &mut String, input: MetricInput<'_>) {
    append_runtime_metrics(body, input.runtime, input.plugins);
    append_upstream_configuration(body, input.kubernetes, input.prometheus, input.webhook);
    append_job_metrics(body, input.jobs, input.replicas);
    append_workspace_metrics(body, input.workspace_metrics);
    body.push_str("# EOF\n");
}

fn append_runtime_metrics(
    body: &mut String,
    runtime: &RuntimeSnapshot,
    plugins: PluginRuntimeMetrics,
) {
    body.push_str("# HELP mwc_process_uptime_seconds Process uptime.\n# TYPE mwc_process_uptime_seconds gauge\n");
    let _ = writeln!(
        body,
        "mwc_process_uptime_seconds {}",
        runtime.uptime.as_secs_f64()
    );
    body.push_str("# HELP mwc_process_resident_memory_bytes Resident set size reported by the operating system.\n# TYPE mwc_process_resident_memory_bytes gauge\n");
    let _ = writeln!(
        body,
        "mwc_process_resident_memory_bytes {}",
        runtime.process.resident_bytes
    );
    body.push_str("# HELP mwc_process_virtual_memory_bytes Virtual address space reported by the operating system.\n# TYPE mwc_process_virtual_memory_bytes gauge\n");
    let _ = writeln!(
        body,
        "mwc_process_virtual_memory_bytes {}",
        runtime.process.virtual_bytes
    );
    body.push_str("# HELP mwc_process_threads Operating-system threads in this process.\n# TYPE mwc_process_threads gauge\n");
    let _ = writeln!(body, "mwc_process_threads {}", runtime.process.threads);
    append_allocator_metrics(body, runtime);
    append_memory_metrics(body, runtime, plugins);
    append_http_metrics(body, runtime);
    append_upstream_metrics(body, runtime);
    append_plugin_metrics(body, plugins);
}

fn append_memory_metrics(
    body: &mut String,
    runtime: &RuntimeSnapshot,
    plugins: PluginRuntimeMetrics,
) {
    body.push_str("# HELP mwc_memory_component_bytes Current bounded in-process memory estimates by component.\n# TYPE mwc_memory_component_bytes gauge\n");
    let _ = writeln!(
        body,
        "mwc_memory_component_bytes{{component=\"http_request_buffers\"}} {}",
        runtime.request_buffer_bytes
    );
    let _ = writeln!(
        body,
        "mwc_memory_component_bytes{{component=\"sse_pending_events\"}} {}",
        runtime.sse_buffer_bytes
    );
    let _ = writeln!(
        body,
        "mwc_memory_component_bytes{{component=\"plugin_registry_metadata\"}} {}",
        plugins.registry_metadata_bytes_estimate
    );
    body.push_str("# HELP mwc_memory_limit_bytes Configured in-process buffer limits by component.\n# TYPE mwc_memory_limit_bytes gauge\n");
    for (component, bytes) in [
        ("plugin_upload", 82_u64 * 1024 * 1024),
        ("plugin_api_request", 256_u64 * 1024),
        ("prometheus_response", 1024_u64 * 1024),
    ] {
        let _ = writeln!(
            body,
            "mwc_memory_limit_bytes{{component=\"{component}\"}} {bytes}"
        );
    }
}

fn append_allocator_metrics(body: &mut String, runtime: &RuntimeSnapshot) {
    let Some(allocator) = runtime.allocator else {
        return;
    };
    body.push_str("# HELP mwc_allocator_bytes Bytes reported by jemalloc, split by bounded memory state.\n# TYPE mwc_allocator_bytes gauge\n");
    for (state, value) in [
        ("allocated", allocator.allocated_bytes),
        ("active", allocator.active_bytes),
        ("resident", allocator.resident_bytes),
        ("mapped", allocator.mapped_bytes),
        ("metadata", allocator.metadata_bytes),
        ("retained", allocator.retained_bytes),
    ] {
        let _ = writeln!(body, "mwc_allocator_bytes{{state=\"{state}\"}} {value}");
    }
}

fn append_http_metrics(body: &mut String, runtime: &RuntimeSnapshot) {
    body.push_str("# HELP mwc_http_requests_active Requests currently executing on the public API listener.\n# TYPE mwc_http_requests_active gauge\n");
    let _ = writeln!(body, "mwc_http_requests_active {}", runtime.active_http);
    body.push_str("# HELP mwc_streams_active Long-lived response streams currently connected.\n# TYPE mwc_streams_active gauge\n");
    let _ = writeln!(
        body,
        "mwc_streams_active{{kind=\"sse\"}} {}",
        runtime.active_sse
    );
    body.push_str("# HELP mwc_http_requests_total Requests completed by bounded method, route template, and status class.\n# TYPE mwc_http_requests_total counter\n");
    for series in &runtime.http {
        for (status, count) in &series.requests_by_status {
            let _ = writeln!(
                body,
                "mwc_http_requests_total{{method=\"{}\",route=\"{}\",status_class=\"{status}\"}} {count}",
                series.method,
                label(&series.route)
            );
        }
    }
    append_http_histograms(body, runtime);
    body.push_str("# HELP mwc_http_body_declared_bytes_total Declared HTTP body bytes observed at request and response boundaries.\n# TYPE mwc_http_body_declared_bytes_total counter\n");
    for series in &runtime.http {
        let labels = format!(
            "method=\"{}\",route=\"{}\"",
            series.method,
            label(&series.route)
        );
        let _ = writeln!(
            body,
            "mwc_http_body_declared_bytes_total{{{labels},direction=\"request\"}} {}",
            series.request_bytes_total
        );
        let _ = writeln!(
            body,
            "mwc_http_body_declared_bytes_total{{{labels},direction=\"response\"}} {}",
            series.response_bytes_total
        );
    }
}

fn append_http_histograms(body: &mut String, runtime: &RuntimeSnapshot) {
    body.push_str("# HELP mwc_http_request_duration_seconds Request handling latency.\n# TYPE mwc_http_request_duration_seconds histogram\n");
    for series in &runtime.http {
        let labels = format!(
            "method=\"{}\",route=\"{}\"",
            series.method,
            label(&series.route)
        );
        for (upper, count) in HTTP_DURATION_BUCKETS.iter().zip(series.duration_buckets) {
            let _ = writeln!(
                body,
                "mwc_http_request_duration_seconds_bucket{{{labels},le=\"{upper}\"}} {count}"
            );
        }
        let _ = writeln!(
            body,
            "mwc_http_request_duration_seconds_bucket{{{labels},le=\"+Inf\"}} {}",
            series.duration_count
        );
        let _ = writeln!(
            body,
            "mwc_http_request_duration_seconds_sum{{{labels}}} {}",
            series.duration_sum
        );
        let _ = writeln!(
            body,
            "mwc_http_request_duration_seconds_count{{{labels}}} {}",
            series.duration_count
        );
    }
}

fn append_upstream_metrics(body: &mut String, runtime: &RuntimeSnapshot) {
    body.push_str("# HELP mwc_upstream_requests_active Outbound requests currently waiting on an upstream.\n# TYPE mwc_upstream_requests_active gauge\n");
    body.push_str("# HELP mwc_upstream_requests_total Completed outbound requests by result.\n# TYPE mwc_upstream_requests_total counter\n");
    for upstream in runtime.upstream {
        let name = upstream.kind.name();
        let _ = writeln!(
            body,
            "mwc_upstream_requests_active{{upstream=\"{name}\"}} {}",
            upstream.active
        );
        let _ = writeln!(
            body,
            "mwc_upstream_requests_total{{upstream=\"{name}\",result=\"success\"}} {}",
            upstream.success
        );
        let _ = writeln!(
            body,
            "mwc_upstream_requests_total{{upstream=\"{name}\",result=\"error\"}} {}",
            upstream.error
        );
    }
}

fn append_upstream_configuration(
    body: &mut String,
    kubernetes: bool,
    prometheus: bool,
    webhook: bool,
) {
    body.push_str("# HELP mwc_upstream_configured Whether an upstream integration is configured for this process.\n# TYPE mwc_upstream_configured gauge\n");
    for (name, configured) in [
        ("kubernetes", kubernetes),
        ("prometheus", prometheus),
        ("webhook", webhook),
    ] {
        let _ = writeln!(
            body,
            "mwc_upstream_configured{{upstream=\"{name}\"}} {}",
            u8::from(configured)
        );
    }
}

fn append_plugin_metrics(body: &mut String, plugins: PluginRuntimeMetrics) {
    body.push_str("# HELP mwc_plugins Plugins in the hot-reloaded runtime registry.\n# TYPE mwc_plugins gauge\n");
    for (state, count) in [
        ("loaded", plugins.loaded),
        ("enabled", plugins.enabled),
        ("executable", plugins.executable),
    ] {
        let _ = writeln!(body, "mwc_plugins{{state=\"{state}\"}} {count}");
    }
    body.push_str("# HELP mwc_plugin_executions_active Wasm plugin executions currently active.\n# TYPE mwc_plugin_executions_active gauge\n");
    let _ = writeln!(
        body,
        "mwc_plugin_executions_active {}",
        plugins.executions_active
    );
    body.push_str("# HELP mwc_plugin_execution_limit Maximum concurrent Wasm plugin executions.\n# TYPE mwc_plugin_execution_limit gauge\n");
    let _ = writeln!(
        body,
        "mwc_plugin_execution_limit {}",
        plugins.execution_limit
    );
}

fn append_job_metrics(body: &mut String, jobs: &crate::storage::JobCounts, replicas: u16) {
    body.push_str("# HELP mwc_jobs Durable background jobs by state.\n# TYPE mwc_jobs gauge\n");
    for (status, count) in [
        ("pending", jobs.pending),
        ("running", jobs.running),
        ("completed", jobs.completed),
        ("failed", jobs.failed),
    ] {
        let _ = writeln!(body, "mwc_jobs{{status=\"{status}\"}} {count}");
    }
    body.push_str("# HELP mwc_jobs_pending Durable jobs waiting to be claimed.\n# TYPE mwc_jobs_pending gauge\n");
    let _ = writeln!(body, "mwc_jobs_pending {}", jobs.pending);
    body.push_str("# HELP mwc_jobs_oldest_pending_age_seconds Age of the oldest pending durable job. Zero when the queue is empty.\n# TYPE mwc_jobs_oldest_pending_age_seconds gauge\n");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default();
    let oldest_pending_age = jobs
        .oldest_pending_created_at
        .map(|created_at| now.saturating_sub(created_at))
        .unwrap_or_default();
    let _ = writeln!(
        body,
        "mwc_jobs_oldest_pending_age_seconds {oldest_pending_age}"
    );
    body.push_str("# HELP mwc_jobs_max_active_attempts Maximum attempt count among pending or running durable jobs.\n# TYPE mwc_jobs_max_active_attempts gauge\n");
    let _ = writeln!(
        body,
        "mwc_jobs_max_active_attempts {}",
        jobs.max_active_attempts
    );
    body.push_str("# HELP mwc_configured_replicas Control-plane replicas configured for this installation.\n# TYPE mwc_configured_replicas gauge\n");
    let _ = writeln!(body, "mwc_configured_replicas {replicas}");
}

fn append_workspace_metrics(body: &mut String, metrics: &crate::storage::WorkspaceMetrics) {
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
    body.push_str("# HELP mwc_user_workspaces Workspaces per owner and lifecycle state.\n# TYPE mwc_user_workspaces gauge\n# HELP mwc_user_resource_requested Requested resources per workspace owner.\n# TYPE mwc_user_resource_requested gauge\n");
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
    let separator = if prefix.is_empty() { "" } else { "," };
    for (resource, unit, value) in [
        ("cpu", "millicores", resources.cpu_millis),
        ("memory", "mebibytes", resources.memory_mib),
        ("gpu", "devices", u64::from(resources.gpu_count)),
        ("disk", "gibibytes", resources.disk_gib),
    ] {
        let _ = writeln!(
            body,
            "{metric}{{{prefix}{separator}resource=\"{resource}\",unit=\"{unit}\"}} {value}"
        );
    }
}

fn label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}
