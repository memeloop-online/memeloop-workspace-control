use std::{net::SocketAddr, sync::Arc};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use memeloop_workspace_control::{
    api::{AppState, internal_router, router},
    config::{AppConfig, InstallationId},
    quota::Resources,
    storage::{CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate, Database},
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::{AccessMode, WorkspaceObservation},
};
use serde_json::Value;
use tower::ServiceExt;

const USER_TOKEN: &str = "web-shell-user-token-000000000000000000000000";
const INTERNAL_TOKEN: &str = "web-shell-internal-token-000000000000000000";

#[tokio::test]
async fn web_shell_ticket_is_ready_only_scoped_and_consumed_once() {
    let installation_id = "shell-test".parse::<InstallationId>().unwrap();
    let database = Database::connect("sqlite::memory:", installation_id.clone())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
        .upsert_image_policy("registry.example/workspace:1", true, 99)
        .await
        .unwrap();
    let user = database
        .create_user("Shell User", USER_TOKEN, true, 100)
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Shell Org".to_owned(),
                owner_user_id: user.user_id,
            },
            101,
        )
        .await
        .unwrap();
    let template = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(organization.id),
                yaml: WorkspaceTemplateDocument::new(
                    "Web shell",
                    WorkspaceTemplateSpec::standard(
                        "registry.example/workspace:1",
                        AccessMode::Public,
                        Resources {
                            cpu_millis: 500,
                            memory_mib: 512,
                            gpu_count: 0,
                            disk_gib: 5,
                        },
                    ),
                )
                .to_yaml()
                .unwrap(),
            },
            false,
            101,
        )
        .await
        .unwrap();
    let workspace = database
        .create_workspace(
            CreateWorkspace {
                organization_id: organization.id,
                owner_id: user.user_id,
                name: "shell".to_owned(),
                template_id: template.id,
                resources: None,
                organization_injection_refs: None,
                user_injection_refs: None,
            },
            false,
            user.user_id,
            102,
        )
        .await
        .unwrap();
    database
        .record_workspace_observation(workspace.id, WorkspaceObservation::Ready, user.user_id, 103)
        .await
        .unwrap();

    let mut state = AppState::new(
        AppConfig {
            installation_id,
            listen_address: SocketAddr::from(([127, 0, 0, 1], 0)),
            database_url: "sqlite::memory:".to_owned(),
            replica_count: 1,
            instance_id: "test".to_owned(),
            ssh_public_host: None,
            internal_ssh_host: None,
            web_shell_public_origin: Some("https://shell.example.com".to_owned()),
            prometheus_url: None,
            plugin_dir: None,
        },
        database,
    );
    state.set_internal_auth_token(INTERNAL_TOKEN);
    let app = router(Arc::new(state.clone()));
    state.trust_internal_network();
    let internal_app = internal_router(Arc::new(state));

    let response = app
        .clone()
        .oneshot(
            Request::post(format!(
                "/api/v1/workspaces/{}/web-shell-tickets",
                workspace.id
            ))
            .header("authorization", format!("Bearer {USER_TOKEN}"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let ticket = body["ticket"].as_str().unwrap();
    assert!(ticket.len() >= 32);
    assert!(
        body["web_shell_url"]
            .as_str()
            .unwrap()
            .starts_with("https://shell.example.com/shell/")
    );
    assert!(body["web_shell_url"].as_str().unwrap().contains(ticket));

    let authorize = || {
        Request::get("/api/v1/internal/web-shell/authorize")
            .header("authorization", format!("Bearer {INTERNAL_TOKEN}"))
            .header("x-mwc-workspace-id", workspace.id.to_string())
            .header(
                "x-forwarded-uri",
                format!("/shell/{}/ws?ticket={ticket}", workspace.short_id),
            )
            .body(Body::empty())
            .unwrap()
    };
    let page = internal_app
        .clone()
        .oneshot(
            Request::get("/api/v1/internal/web-shell/authorize")
                .header(
                    "x-forwarded-uri",
                    format!("/shell/{}/?ticket={ticket}", workspace.short_id),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    assert!(!page.headers().contains_key("x-mwc-user-id"));

    let first = internal_app.clone().oneshot(authorize()).await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(first.headers()["x-mwc-user-id"], user.user_id.to_string());
    let replay = internal_app.oneshot(authorize()).await.unwrap();
    assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn ticket_storage_rejects_expired_or_wrong_workspace_without_leaking_debug_value() {
    let database = Database::connect("sqlite::memory:", "ticket-test".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let organization_id = uuid::Uuid::now_v7();
    let workspace_id = uuid::Uuid::now_v7();
    let user_id = uuid::Uuid::now_v7();
    let issued = database
        .issue_web_shell_ticket(organization_id, workspace_id, user_id, 100, 60)
        .await
        .unwrap();
    assert!(!format!("{issued:?}").contains(&issued.ticket));
    assert!(
        database
            .consume_web_shell_ticket(&issued.ticket, uuid::Uuid::now_v7(), 101)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        database
            .consume_web_shell_ticket(&issued.ticket, workspace_id, 161)
            .await
            .unwrap()
            .is_none()
    );
}
