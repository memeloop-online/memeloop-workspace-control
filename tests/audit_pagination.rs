use std::{net::SocketAddr, sync::Arc};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use memeloop_workspace_control::{
    api::{AppState, router},
    auth::Role,
    config::{AppConfig, InstallationId},
    storage::{CreateOrganization, Database},
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN_TOKEN: &str = "audit-admin-token-0000000000000000000000000";
const MEMBER_TOKEN: &str = "audit-member-token-000000000000000000000000";
const SYSTEM_TOKEN: &str = "audit-system-token-000000000000000000000000";

#[tokio::test]
async fn audit_api_pages_filters_and_enforces_organization_rbac() {
    let installation_id: InstallationId = "audit-page".parse().unwrap();
    let database = Database::connect("sqlite::memory:", installation_id.clone())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let admin = database
        .create_user("Audit Admin", ADMIN_TOKEN, false, 1)
        .await
        .unwrap();
    let member = database
        .create_user("Audit Member", MEMBER_TOKEN, false, 2)
        .await
        .unwrap();
    database
        .create_user("System Admin", SYSTEM_TOKEN, true, 3)
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Audit Organization".to_owned(),
                owner_user_id: admin.user_id,
            },
            3,
        )
        .await
        .unwrap();
    database
        .upsert_membership(organization.id, member.user_id, Role::Member, 4)
        .await
        .unwrap();
    let workspace_id = Uuid::now_v7();
    for index in 0..5 {
        database
            .record_audit(
                Some(admin.user_id),
                Some(organization.id),
                (index == 3).then_some(workspace_id),
                &format!("test.action.{index}"),
                json!({"searchable": format!("marker-{index}")}),
                100 + index,
            )
            .await
            .unwrap();
    }
    database
        .record_audit(
            Some(admin.user_id),
            None,
            None,
            "user.profile.update",
            json!({"fields": ["display_name"]}),
            200,
        )
        .await
        .unwrap();
    let app = router(Arc::new(AppState::new(
        AppConfig {
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
        },
        database,
    )));

    let first = get_json(
        &app,
        &format!(
            "/api/v1/audit?organization_id={}&limit=2&offset=0",
            organization.id
        ),
        ADMIN_TOKEN,
        StatusCode::OK,
    )
    .await;
    assert_eq!(first["items"].as_array().unwrap().len(), 2);
    assert_eq!(first["items"][0]["action"], "test.action.4");
    assert_eq!(first["next_offset"], 2);

    let second = get_json(
        &app,
        &format!(
            "/api/v1/audit?organization_id={}&limit=2&offset=2",
            organization.id
        ),
        ADMIN_TOKEN,
        StatusCode::OK,
    )
    .await;
    assert_eq!(second["items"][0]["action"], "test.action.2");
    assert_eq!(second["next_offset"], 4);

    for (suffix, expected) in [
        ("action=test.action.1".to_owned(), 1),
        (format!("actor={}", admin.user_id), 5),
        (format!("workspace={workspace_id}"), 1),
        ("q=marker-4".to_owned(), 1),
    ] {
        let filtered = get_json(
            &app,
            &format!("/api/v1/audit?organization_id={}&{suffix}", organization.id),
            ADMIN_TOKEN,
            StatusCode::OK,
        )
        .await;
        assert_eq!(
            filtered["items"].as_array().unwrap().len(),
            expected,
            "{suffix}"
        );
        assert!(filtered["next_offset"].is_null());
    }

    let forbidden = get_json(
        &app,
        &format!("/api/v1/audit?organization_id={}", organization.id),
        MEMBER_TOKEN,
        StatusCode::FORBIDDEN,
    )
    .await;
    assert_eq!(forbidden["error"]["code"], "forbidden");

    let organization_admin_global = get_json(
        &app,
        "/api/v1/audit?limit=10",
        ADMIN_TOKEN,
        StatusCode::FORBIDDEN,
    )
    .await;
    assert_eq!(organization_admin_global["error"]["code"], "forbidden");

    let global = get_json(
        &app,
        "/api/v1/audit?limit=10&action=user.profile.update",
        SYSTEM_TOKEN,
        StatusCode::OK,
    )
    .await;
    assert_eq!(global["items"].as_array().unwrap().len(), 1);
    assert_eq!(global["items"][0]["action"], "user.profile.update");
}

async fn get_json(app: &axum::Router, uri: &str, token: &str, status: StatusCode) -> Value {
    let response = app
        .clone()
        .oneshot(
            Request::get(uri)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), status);
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
