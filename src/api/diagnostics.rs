use std::{sync::Arc, time::Duration};

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use super::AppState;

#[derive(Debug, Deserialize)]
pub(super) struct CpuProfileQuery {
    #[serde(default = "default_profile_seconds")]
    seconds: u64,
}

#[derive(Debug, Serialize)]
pub(super) struct ProcessDiagnostics {
    process: crate::observability::ProcessSnapshot,
    allocator: Option<crate::observability::AllocatorSnapshot>,
    active_http_requests: u64,
    active_sse_streams: u64,
    cpu_profile_active: bool,
}

pub(super) async fn process(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Err(status) = authorize(&state, &headers) {
        return status.into_response();
    }
    let snapshot = state.observability.snapshot();
    Json(ProcessDiagnostics {
        process: snapshot.process,
        allocator: snapshot.allocator,
        active_http_requests: snapshot.active_http,
        active_sse_streams: snapshot.active_sse,
        cpu_profile_active: snapshot.cpu_profile_active,
    })
    .into_response()
}

pub(super) async fn cpu_profile(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<CpuProfileQuery>,
) -> Response {
    if let Err(status) = authorize(&state, &headers) {
        return status.into_response();
    }
    if !(1..=30).contains(&query.seconds) {
        return (StatusCode::BAD_REQUEST, "seconds must be between 1 and 30").into_response();
    }
    let Some(profile_guard) = state.observability.try_begin_cpu_profile() else {
        return (StatusCode::CONFLICT, "a CPU profile is already running").into_response();
    };
    cpu_profile_platform(profile_guard, query.seconds).await
}

pub(super) async fn heap_profile(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = authorize(&state, &headers) {
        return status.into_response();
    }
    heap_profile_platform().await
}

fn authorize(state: &AppState, headers: &HeaderMap) -> Result<(), StatusCode> {
    if !state.diagnostics_enabled {
        return Err(StatusCode::NOT_FOUND);
    }
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if !state.internal_caller_allowed(token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn cpu_profile_platform(
    profile_guard: crate::observability::CpuProfileGuard,
    seconds: u64,
) -> Response {
    use pprof::protos::Message;

    let result = tokio::task::spawn_blocking(move || {
        let _profile_guard = profile_guard;
        let guard = pprof::ProfilerGuardBuilder::default()
            .frequency(99)
            .blocklist(&["libc", "libgcc", "pthread", "vdso"])
            .build()?;
        std::thread::sleep(Duration::from_secs(seconds));
        let profile = guard.report().build()?.pprof()?;
        let mut body = Vec::new();
        profile.encode(&mut body)?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(body)
    })
    .await;
    match result {
        Ok(Ok(body)) => profile_response(body, "mwc-cpu.pb"),
        Ok(Err(error)) => {
            tracing::error!(%error, "CPU profile capture failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "CPU profile capture failed",
            )
                .into_response()
        }
        Err(error) => {
            tracing::error!(%error, "CPU profile task failed");
            (StatusCode::SERVICE_UNAVAILABLE, "CPU profile task failed").into_response()
        }
    }
}

#[cfg(not(target_os = "linux"))]
async fn cpu_profile_platform(
    _profile_guard: crate::observability::CpuProfileGuard,
    _seconds: u64,
) -> Response {
    (StatusCode::NOT_IMPLEMENTED, "CPU profiling requires Linux").into_response()
}

#[cfg(target_os = "linux")]
async fn heap_profile_platform() -> Response {
    let Some(control) = jemalloc_pprof::PROF_CTL.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "heap profiling is unavailable",
        )
            .into_response();
    };
    let mut control = control.lock().await;
    if !control.activated() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "heap profiling is inactive",
        )
            .into_response();
    }
    match control.dump_pprof() {
        Ok(body) => profile_response(body, "mwc-heap.pb.gz"),
        Err(error) => {
            tracing::error!(%error, "heap profile capture failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "heap profile capture failed",
            )
                .into_response()
        }
    }
}

#[cfg(not(target_os = "linux"))]
async fn heap_profile_platform() -> Response {
    (StatusCode::NOT_IMPLEMENTED, "heap profiling requires Linux").into_response()
}

fn profile_response(body: Vec<u8>, filename: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::CACHE_CONTROL, "no-store"),
            (
                header::CONTENT_DISPOSITION,
                if filename.ends_with(".gz") {
                    "attachment; filename=\"mwc-heap.pb.gz\""
                } else {
                    "attachment; filename=\"mwc-cpu.pb\""
                },
            ),
        ],
        body,
    )
        .into_response()
}

const fn default_profile_seconds() -> u64 {
    10
}
