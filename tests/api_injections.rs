use std::{net::SocketAddr, sync::Arc};

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use http_body_util::BodyExt;
use memeloop_workspace_control::{
    api::{AppState, router},
    config::AppConfig,
    crypto::EnvelopeCipher,
    storage::Database,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

const TOKEN: &str = "injection-user-00000000000000000000000000";

async fn app(with_cipher: bool) -> (Router, Uuid) {
    let installation_id = "injection-api".parse().unwrap();
    let database = Database::connect("sqlite::memory:", installation_id)
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let user = database
        .create_user("Injection User", TOKEN, false, 1)
        .await
        .unwrap();
    let config = AppConfig {
        installation_id: "injection-api".parse().unwrap(),
        listen_address: SocketAddr::from(([127, 0, 0, 1], 0)),
        database_url: "sqlite::memory:".to_owned(),
        replica_count: 1,
        instance_id: "test".to_owned(),
        ssh_public_host: None,
        web_shell_public_origin: None,
    };
    let state = if with_cipher {
        AppState::with_cipher(
            config,
            database,
            EnvelopeCipher::from_base64(&STANDARD.encode([9_u8; 32])).unwrap(),
        )
    } else {
        AppState::new(config, database)
    };
    (router(Arc::new(state)), user.user_id)
}

fn request(method: Method, uri: &str, key: Option<&str>, body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("Authorization", format!("Bearer {TOKEN}"));
    if let Some(key) = key {
        builder = builder.header("Idempotency-Key", key);
    }
    if body.is_some() {
        builder = builder.header("Content-Type", "application/json");
    }
    builder
        .body(Body::from(
            body.map_or_else(String::new, |value| value.to_string()),
        ))
        .unwrap()
}

async fn response_body(response: axum::response::Response) -> (StatusCode, Vec<u8>) {
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, bytes.to_vec())
}

#[tokio::test]
async fn injection_api_is_write_only_versioned_and_idempotent() {
    let (app, user_id) = app(true).await;
    let secret = "line one\n\n  line two\n";
    let item = json!({
        "key": "credentials",
        "kind": "secret_file",
        "target": "/workspace/.config/credentials",
        "value": {"encoding": "utf8", "value": secret},
        "sensitive": true,
        "locked": false,
        "version": 999,
        "file_mode": 384,
        "owner": "workspace",
        "group": "workspace",
        "template_selector": null,
        "labels": {"environment": "test"}
    });
    let uri = format!("/api/v1/injections/user/{user_id}/credentials");
    let first = app
        .clone()
        .oneshot(request(
            Method::PUT,
            &uri,
            Some("injection-request-1"),
            Some(item.clone()),
        ))
        .await
        .unwrap();
    let (first_status, first_body) = response_body(first).await;
    assert_eq!(first_status, StatusCode::OK);
    assert!(!String::from_utf8_lossy(&first_body).contains(secret));
    let summary: Value = serde_json::from_slice(&first_body).unwrap();
    assert_eq!(summary["version"], 1);

    let replay = app
        .clone()
        .oneshot(request(
            Method::PUT,
            &uri,
            Some("injection-request-1"),
            Some(item),
        ))
        .await
        .unwrap();
    let (replay_status, replay_body) = response_body(replay).await;
    assert_eq!(replay_status, first_status);
    assert_eq!(replay_body, first_body);

    let list = app
        .clone()
        .oneshot(request(
            Method::GET,
            &format!("/api/v1/injections/user/{user_id}"),
            None,
            None,
        ))
        .await
        .unwrap();
    let (list_status, list_body) = response_body(list).await;
    assert_eq!(list_status, StatusCode::OK);
    assert!(!String::from_utf8_lossy(&list_body).contains(secret));
    let summaries: Value = serde_json::from_slice(&list_body).unwrap();
    assert_eq!(summaries.as_array().unwrap().len(), 1);

    let explicitly_empty = app
        .clone()
        .oneshot(request(
            Method::POST,
            "/api/v1/injections/preview",
            None,
            Some(json!({
                "organization_id": null,
                "user_id": user_id,
                "workspace_id": null,
                "organization_injection_refs": [],
                "user_injection_refs": [],
                "inline_workspace_injections": []
            })),
        ))
        .await
        .unwrap();
    let (empty_status, empty_body) = response_body(explicitly_empty).await;
    assert_eq!(empty_status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<Value>(&empty_body).unwrap(),
        json!([])
    );

    let missing_reference = app
        .clone()
        .oneshot(request(
            Method::POST,
            "/api/v1/injections/preview",
            None,
            Some(json!({
                "organization_id": null,
                "user_id": user_id,
                "workspace_id": null,
                "user_injection_refs": ["missing"],
                "inline_workspace_injections": []
            })),
        ))
        .await
        .unwrap();
    assert_eq!(missing_reference.status(), StatusCode::BAD_REQUEST);

    let preview = app
        .oneshot(request(
            Method::POST,
            "/api/v1/injections/preview",
            None,
            Some(json!({
                "organization_id": null,
                "user_id": user_id,
                "workspace_id": null,
                "inline_workspace_injections": [{
                    "key": "credentials",
                    "kind": "secret_file",
                    "target": "/workspace/.config/credentials",
                    "value": {"encoding": "utf8", "value": "inline-secret"},
                    "sensitive": true,
                    "locked": false,
                    "version": 0,
                    "file_mode": 384,
                    "owner": "workspace",
                    "group": "workspace",
                    "template_selector": null,
                    "labels": {}
                }]
            })),
        ))
        .await
        .unwrap();
    let (preview_status, preview_body) = response_body(preview).await;
    assert_eq!(preview_status, StatusCode::OK);
    let preview_json: Value = serde_json::from_slice(&preview_body).unwrap();
    assert_eq!(preview_json[0]["source"], "workspace");
    assert!(!String::from_utf8_lossy(&preview_body).contains("inline-secret"));
}

#[tokio::test]
async fn injection_write_requires_configured_encryption_key() {
    let (app, user_id) = app(false).await;
    let response = app
        .oneshot(request(
            Method::PUT,
            &format!("/api/v1/injections/user/{user_id}/key"),
            Some("request-1"),
            Some(json!({
                "key": "key",
                "kind": "environment_variable",
                "target": "TOKEN",
                "value": {"encoding": "utf8", "value": "secret"},
                "sensitive": true,
                "locked": false,
                "version": 0,
                "file_mode": null,
                "owner": null,
                "group": null,
                "template_selector": null,
                "labels": {}
            })),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}
