use std::{net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use futures::StreamExt;
use memeloop_workspace_control::{
    api::{AppState, router},
    config::AppConfig,
    events::NewEvent,
    storage::Database,
};
use tower::ServiceExt;
use uuid::Uuid;

const TOKEN: &str = "event-admin-0000000000000000000000000000";

#[tokio::test]
async fn sse_resumes_after_durable_last_event_id_and_filters_organization() {
    let installation_id = "event-test".parse().unwrap();
    let database = Database::connect("sqlite::memory:", installation_id)
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
        .create_user("Event Admin", TOKEN, true, 1)
        .await
        .unwrap();
    let organization_id = Uuid::now_v7();
    let other_organization = Uuid::now_v7();
    let first = database
        .append_event(
            NewEvent {
                organization_id,
                workspace_id: None,
                kind: "workspace.created".to_owned(),
                payload: serde_json::json!({"sequence": 1}),
            },
            10,
        )
        .await
        .unwrap();
    database
        .append_event(
            NewEvent {
                organization_id: other_organization,
                workspace_id: None,
                kind: "workspace.created".to_owned(),
                payload: serde_json::json!({"sequence": 999}),
            },
            11,
        )
        .await
        .unwrap();
    let second = database
        .append_event(
            NewEvent {
                organization_id,
                workspace_id: None,
                kind: "workspace.ready".to_owned(),
                payload: serde_json::json!({"sequence": 2}),
            },
            12,
        )
        .await
        .unwrap();
    let config = AppConfig {
        installation_id: "event-test".parse().unwrap(),
        listen_address: SocketAddr::from(([127, 0, 0, 1], 0)),
        database_url: "sqlite::memory:".to_owned(),
        replica_count: 1,
        instance_id: "test".to_owned(),
        ssh_public_host: None,
        web_shell_public_origin: None,
    };
    let app = router(Arc::new(AppState::new(config, database)));
    let response = app
        .oneshot(
            Request::get(format!("/api/v1/events?organization_id={organization_id}"))
                .header("Authorization", format!("Bearer {TOKEN}"))
                .header("Last-Event-ID", first.id.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    let chunk = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let text = String::from_utf8(chunk.to_vec()).unwrap();
    assert!(text.contains(&format!("id: {}", second.id)));
    assert!(text.contains("event: workspace.ready"));
    assert!(text.contains("\"sequence\":2"));
    assert!(!text.contains("999"));
}
