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
    config::{AppConfig, InstallationId},
    storage::Database,
};
use serde_json::{Value, json};
use tower::ServiceExt;

const PRIMARY_TOKEN: &str = "settings-primary-token-0000000000000000000000";
const OTHER_TOKEN: &str = "settings-other-token-000000000000000000000000";

async fn app() -> (Router, Database) {
    let installation_id: InstallationId = "settings-api".parse().unwrap();
    let database = Database::connect("sqlite::memory:", installation_id.clone())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
        .create_user("Primary User", PRIMARY_TOKEN, false, 1)
        .await
        .unwrap();
    database
        .create_user("Other User", OTHER_TOKEN, false, 2)
        .await
        .unwrap();
    let config = AppConfig {
        installation_id,
        listen_address: SocketAddr::from(([127, 0, 0, 1], 0)),
        database_url: "sqlite::memory:".to_owned(),
        replica_count: 1,
        instance_id: "test".to_owned(),
        ssh_public_host: None,
        internal_ssh_host: None,
        web_shell_public_origin: None,
        port_mapping_public_domain: None,
        prometheus_url: None,
        plugin_dir: None,
    };
    (
        router(Arc::new(AppState::new(config, database.clone()))),
        database,
    )
}

#[tokio::test]
async fn profile_is_self_scoped_persistent_and_uses_a_stable_generated_avatar() {
    let (app, _) = app().await;
    let initial = json_response(
        app.clone()
            .oneshot(request(
                Method::GET,
                "/api/v1/me/profile",
                PRIMARY_TOKEN,
                None,
            ))
            .await
            .unwrap(),
        StatusCode::OK,
    )
    .await;
    assert_eq!(initial["display_name"], "Primary User");
    let generated = initial["avatar_url"].as_str().unwrap();
    assert!(generated.starts_with("data:image/svg+xml;base64,"));

    let updated = json_response(
        app.clone()
            .oneshot(request(
                Method::PUT,
                "/api/v1/me/profile",
                PRIMARY_TOKEN,
                Some(json!({
                    "display_name": "  主要用户  ",
                    "avatar_url": generated
                })),
            ))
            .await
            .unwrap(),
        StatusCode::OK,
    )
    .await;
    assert_eq!(updated["display_name"], "主要用户");
    assert_eq!(updated["avatar_url"], generated);

    let uploaded_avatar = format!(
        "data:image/png;base64,{}",
        STANDARD.encode(b"\x89PNG\r\n\x1a\nuploaded-avatar")
    );
    let custom = json_response(
        app.clone()
            .oneshot(request(
                Method::PUT,
                "/api/v1/me/profile",
                PRIMARY_TOKEN,
                Some(json!({
                    "display_name": "主要用户",
                    "avatar_url": uploaded_avatar
                })),
            ))
            .await
            .unwrap(),
        StatusCode::OK,
    )
    .await;
    assert_eq!(custom["avatar_url"], uploaded_avatar);

    let invalid = app
        .clone()
        .oneshot(request(
            Method::PUT,
            "/api/v1/me/profile",
            PRIMARY_TOKEN,
            Some(json!({
                "display_name": "主要用户",
                "avatar_url": "https://remote.example.test/avatar.png"
            })),
        ))
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let other = json_response(
        app.clone()
            .oneshot(request(
                Method::PUT,
                "/api/v1/me/profile",
                OTHER_TOKEN,
                Some(json!({"display_name": "Other Updated", "avatar_url": null})),
            ))
            .await
            .unwrap(),
        StatusCode::OK,
    )
    .await;
    assert_eq!(other["display_name"], "Other Updated");

    let primary_again = json_response(
        app.oneshot(request(
            Method::GET,
            "/api/v1/me/profile",
            PRIMARY_TOKEN,
            None,
        ))
        .await
        .unwrap(),
        StatusCode::OK,
    )
    .await;
    assert_eq!(primary_again["display_name"], "主要用户");
}

#[tokio::test]
async fn api_keys_rotate_without_ever_returning_stored_tokens() {
    let (app, database) = app().await;
    let initial = json_response(
        app.clone()
            .oneshot(request(
                Method::GET,
                "/api/v1/me/api-keys",
                PRIMARY_TOKEN,
                None,
            ))
            .await
            .unwrap(),
        StatusCode::OK,
    )
    .await;
    assert_eq!(initial.as_array().unwrap().len(), 1);
    let initial_key_id = initial[0]["id"].as_str().unwrap();
    assert!(initial[0].get("token").is_none());

    let created = json_response(
        app.clone()
            .oneshot(request(
                Method::POST,
                "/api/v1/me/api-keys",
                PRIMARY_TOKEN,
                Some(json!({
                    "name": "Windows workstation",
                    "scopes": ["read_workspace", "manage_api_keys"],
                    "expires_at": 1_800_000_000i64
                })),
            ))
            .await
            .unwrap(),
        StatusCode::CREATED,
    )
    .await;
    let rotated_token = created["token"].as_str().unwrap().to_owned();
    assert!(rotated_token.starts_with("mwc_"));
    assert_eq!(created["name"], "Windows workstation");
    assert_eq!(
        created["scopes"],
        json!(["manage_api_keys", "read_workspace"])
    );
    assert_eq!(created["expires_at"], 1_800_000_000i64);
    assert!(created["prefix"].as_str().unwrap().ends_with('…'));

    let listed = json_response(
        app.clone()
            .oneshot(request(
                Method::GET,
                "/api/v1/me/api-keys",
                &rotated_token,
                None,
            ))
            .await
            .unwrap(),
        StatusCode::OK,
    )
    .await;
    assert_eq!(listed.as_array().unwrap().len(), 2);
    let serialized_list = serde_json::to_string(&listed).unwrap();
    assert!(!serialized_list.contains(&rotated_token));
    assert!(!serialized_list.contains(PRIMARY_TOKEN));

    let first_last_used = key_last_used(&database, created["id"].as_str().unwrap()).await;
    let repeated_login = app
        .clone()
        .oneshot(request(Method::GET, "/api/v1/me", &rotated_token, None))
        .await
        .unwrap();
    assert_eq!(repeated_login.status(), StatusCode::OK);
    assert_eq!(
        key_last_used(&database, created["id"].as_str().unwrap()).await,
        first_last_used,
        "authentication inside the throttle window must not update last_used_at"
    );

    let revoked = app
        .clone()
        .oneshot(request(
            Method::DELETE,
            &format!("/api/v1/me/api-keys/{initial_key_id}"),
            &rotated_token,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);

    let old_login = app
        .clone()
        .oneshot(request(Method::GET, "/api/v1/me", PRIMARY_TOKEN, None))
        .await
        .unwrap();
    assert_eq!(old_login.status(), StatusCode::UNAUTHORIZED);

    let last_key = app
        .clone()
        .oneshot(request(
            Method::DELETE,
            &format!("/api/v1/me/api-keys/{}", created["id"].as_str().unwrap()),
            &rotated_token,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(last_key.status(), StatusCode::CONFLICT);

    let missing_key = app
        .oneshot(request(
            Method::DELETE,
            &format!("/api/v1/me/api-keys/{}", uuid::Uuid::now_v7()),
            &rotated_token,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(missing_key.status(), StatusCode::NOT_FOUND);

    let snapshot = database.export_snapshot(100).await.unwrap();
    let snapshot_json = serde_json::to_string(&snapshot).unwrap();
    assert!(!snapshot_json.contains(&rotated_token));
    assert!(!snapshot_json.contains(PRIMARY_TOKEN));
    assert!(snapshot_json.contains("user.api_key.create"));
    assert!(snapshot_json.contains("user.api_key.revoke"));
}

async fn key_last_used(database: &Database, key_id: &str) -> Value {
    database.export_snapshot(100).await.unwrap().tables["user_api_keys"]
        .iter()
        .find(|row| row["id"] == key_id)
        .unwrap()["last_used_at"]
        .clone()
}

fn request(method: Method, uri: &str, token: &str, body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    builder
        .body(Body::from(
            body.map_or_else(String::new, |value| value.to_string()),
        ))
        .unwrap()
}

async fn json_response(response: axum::response::Response, expected: StatusCode) -> Value {
    assert_eq!(response.status(), expected);
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
