use std::{net::SocketAddr, str::FromStr, sync::Arc};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::ServiceExt;

use super::*;

async fn test_state() -> AppState {
    let installation_id = InstallationId::from_str("test-a").unwrap();
    let database = Database::connect("sqlite::memory:", installation_id.clone())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    AppState::new(
        AppConfig {
            installation_id,
            listen_address: SocketAddr::from(([127, 0, 0, 1], 0)),
            database_url: "sqlite::memory:".to_owned(),
            replica_count: 1,
            instance_id: "test".to_owned(),
            ssh_public_host: None,
            internal_ssh_host: None,
            web_shell_public_origin: None,
            prometheus_url: None,
            plugin_dir: None,
        },
        database,
    )
}

async fn test_app() -> Router {
    router(Arc::new(test_state().await))
}

#[tokio::test]
async fn health_endpoint_is_available() {
    let app = test_app().await;
    for path in ["/livez", "/healthz", "/readyz"] {
        let response = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
    }
}

#[tokio::test]
async fn diagnostics_require_the_flag_and_internal_bearer_token() {
    let disabled = internal_router(Arc::new(test_state().await));
    let response = disabled
        .oneshot(
            Request::get("/diagnostics/process")
                .header("authorization", "Bearer aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let mut state = test_state().await;
    state.set_internal_auth_token("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    state.enable_diagnostics();
    let app = internal_router(Arc::new(state));
    let unauthorized = app
        .clone()
        .oneshot(
            Request::get("/diagnostics/process")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let authorized = app
        .clone()
        .oneshot(
            Request::get("/diagnostics/process")
                .header("authorization", "Bearer aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(authorized.status(), StatusCode::OK);
    assert_eq!(authorized.headers()["content-type"], "application/json");
    let invalid_profile = app
        .oneshot(
            Request::get("/debug/pprof/profile?seconds=0")
                .header("authorization", "Bearer aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid_profile.status(), StatusCode::BAD_REQUEST);
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn release_diagnostics_return_pprof_compatible_payloads() {
    let mut state = test_state().await;
    state.set_internal_auth_token("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    state.enable_diagnostics();
    let app = internal_router(Arc::new(state));
    let cpu = app
        .clone()
        .oneshot(
            Request::get("/debug/pprof/profile?seconds=1")
                .header("authorization", "Bearer aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cpu.status(), StatusCode::OK);
    assert_eq!(cpu.headers()["content-type"], "application/octet-stream");
    assert!(cpu.into_body().collect().await.unwrap().to_bytes().len() > 64);

    jemalloc_pprof::activate_jemalloc_profiling().await;
    let sampled_allocation = vec![0_u8; 4 * 1024 * 1024];
    let heap = app
        .oneshot(
            Request::get("/debug/pprof/heap")
                .header("authorization", "Bearer aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    jemalloc_pprof::deactivate_jemalloc_profiling().await;
    assert_eq!(heap.status(), StatusCode::OK);
    assert_eq!(heap.headers()["content-type"], "application/octet-stream");
    assert!(heap.into_body().collect().await.unwrap().to_bytes().len() > 64);
    std::hint::black_box(sampled_allocation);
}

#[tokio::test]
async fn embedded_ui_and_hashed_assets_are_served() {
    let app = test_app().await;
    let response = app
        .clone()
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-cache");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(html.contains("Memeloop Workspace Control"));
    let asset_path = html
        .split("src=\"")
        .nth(1)
        .and_then(|value| value.split('"').next())
        .unwrap();
    let asset = app
        .oneshot(Request::get(asset_path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(asset.status(), StatusCode::OK);
    assert_eq!(
        asset.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
}

#[tokio::test]
async fn public_listener_does_not_expose_internal_routes_or_spa_fallback_for_api_paths() {
    let app = test_app().await;
    for path in [
        "/api/v1/internal/ssh/login-users",
        "/api/v1/internal/web-shell/authorize",
        "/api/v1/does-not-exist",
        "/debug/pprof/profile",
        "/debug/pprof/heap",
        "/diagnostics/process",
    ] {
        let response = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
    let spa_route = app
        .oneshot(Request::get("/workspaces").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(spa_route.status(), StatusCode::OK);
}

#[tokio::test]
async fn info_and_metrics_expose_only_operational_metadata() {
    let app = test_app().await;
    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/system/info")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["installation_id"], "test-a");
    assert_eq!(body["api_version"], "v1");
    assert_eq!(body["database_mode"], "sqlite");
    assert_eq!(body.as_object().unwrap().len(), 3);
    let metrics = app
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(
        metrics.headers()["content-type"],
        "application/openmetrics-text; version=1.0.0; charset=utf-8"
    );
    let text = String::from_utf8(
        metrics
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(text.contains("mwc_http_requests_total"));
    assert!(text.contains("mwc_http_request_duration_seconds_bucket"));
    assert!(text.contains("mwc_process_resident_memory_bytes"));
    assert!(text.contains("mwc_allocator_bytes{state=\"allocated\"}"));
    assert!(text.contains("mwc_upstream_requests_active{upstream=\"prometheus\"}"));
    assert!(text.contains("mwc_plugins{state=\"loaded\"}"));
    assert!(text.contains("mwc_jobs{status=\"pending\"}"));
    assert!(text.contains("# TYPE mwc_workspaces gauge"));
    assert!(text.contains("mwc_resource_requested{resource=\"cpu\",unit=\"millicores\"} 0"));
    assert!(text.ends_with("# EOF\n"));
}

#[tokio::test]
async fn metrics_use_route_templates_instead_of_concrete_identifiers() {
    let app = test_app().await;
    let concrete_id = "018f0f5d-55cc-7f2d-912f-7c4d5d2f8490";
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/workspaces/{concrete_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let metrics = app
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let text = String::from_utf8(
        metrics
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(text.contains("route=\"/api/v1/workspaces/{workspace_id}\""));
    assert!(!text.contains(concrete_id));
}

#[tokio::test]
async fn openapi_document_contains_versioned_api() {
    let response = test_app()
        .await
        .oneshot(
            Request::get("/api/v1/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(body["paths"]["/livez"].is_object());
    assert!(body["paths"]["/readyz"].is_object());
    assert!(body["paths"]["/api/v1/system/info"].is_object());
    assert!(body["paths"]["/api/v1/webhooks"].is_object());
    assert!(body["paths"]["/api/v1/plugins"].is_object());
    assert!(body["paths"]["/api/v1/me/profile"]["put"].is_object());
    assert!(body["paths"]["/api/v1/me/api-keys"]["post"].is_object());
    assert!(
        body["paths"]["/api/v1/me/api-keys"]["post"]["description"]
            .as_str()
            .unwrap()
            .contains("shown only in this response")
    );
    assert!(body["paths"]["/api/v1/audit"]["get"].is_object());
    assert!(body["paths"]["/api/v1/plugins/{plugin_id}/configuration"]["delete"].is_object());
    assert!(body["paths"]["/api/v1/injections/{scope}/{scope_id}/{key}"]["delete"].is_object());
    assert!(
        body["components"]["schemas"]["WorkspaceTemplateSpec"]["properties"]
            .get("environment")
            .is_none()
    );
}
