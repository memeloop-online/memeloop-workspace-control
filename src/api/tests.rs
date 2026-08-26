use std::{net::SocketAddr, str::FromStr, sync::Arc};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::ServiceExt;

use super::*;

async fn test_app() -> Router {
    let installation_id = InstallationId::from_str("test-a").unwrap();
    let database = Database::connect("sqlite::memory:", installation_id.clone())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    router(Arc::new(AppState::new(
        AppConfig {
            installation_id,
            listen_address: SocketAddr::from(([127, 0, 0, 1], 0)),
            database_url: "sqlite::memory:".to_owned(),
            replica_count: 1,
            instance_id: "test".to_owned(),
            ssh_public_host: None,
            web_shell_public_origin: None,
        },
        database,
    )))
}

#[tokio::test]
async fn health_endpoint_is_available() {
    let response = test_app()
        .await
        .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
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
    assert!(text.contains("mwc_jobs{status=\"pending\"}"));
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
    assert!(body["paths"]["/api/v1/system/info"].is_object());
    assert!(body["paths"]["/api/v1/webhooks"].is_object());
}
