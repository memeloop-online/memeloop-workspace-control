use std::{net::SocketAddr, sync::Arc};

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use memeloop_workspace_control::{
    api::{AppState, router},
    config::AppConfig,
    quota::Resources,
    storage::{CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate, Database},
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::{AccessMode, WorkspaceAction, WorkspaceObservation},
};
use serde_json::{Value, json};
use tower::ServiceExt;

const ADMIN_TOKEN: &str = "image-update-admin-000000000000000000000000";
const MEMBER_TOKEN: &str = "image-update-member-000000000000000000000";
const OLD_IMAGE: &str = "registry.example/workspace@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const NEW_IMAGE: &str = "registry.example/workspace@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

async fn app() -> (
    Router,
    Database,
    memeloop_workspace_control::workspaces::Workspace,
) {
    let database = Database::connect("sqlite::memory:", "image-update-api".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
        .upsert_image_policy(OLD_IMAGE, true, 1)
        .await
        .unwrap();
    database
        .upsert_image_policy(NEW_IMAGE, true, 2)
        .await
        .unwrap();
    let admin = database
        .create_user_with_initial_key("Admin", ADMIN_TOKEN, true, memeloop_workspace_control::auth::ApiKeyScope::initial_key_defaults(true), 31_536_000, 1)
        .await
        .unwrap();
    database
        .create_user_with_initial_key("Member", MEMBER_TOKEN, false, memeloop_workspace_control::auth::ApiKeyScope::initial_key_defaults(false), 31_536_000, 1)
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Image update".to_owned(),
                owner_user_id: admin.user_id,
            },
            2,
        )
        .await
        .unwrap();
    let yaml = WorkspaceTemplateDocument::new(
        "Image update",
        WorkspaceTemplateSpec::standard(
            OLD_IMAGE,
            AccessMode::Internal,
            Resources {
                cpu_millis: 1_000,
                memory_mib: 2_048,
                gpu_count: 0,
                disk_gib: 20,
            },
        ),
    )
    .to_yaml()
    .unwrap();
    let template = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(organization.id),
                yaml,
            },
            true,
            3,
        )
        .await
        .unwrap();
    let workspace = database
        .create_workspace(
            CreateWorkspace {
                organization_id: organization.id,
                owner_id: admin.user_id,
                name: "Stopped image update".to_owned(),
                template_id: template.id,
                resources: None,
                organization_injection_refs: None,
                user_injection_refs: None,
            },
            true,
            admin.user_id,
            4,
        )
        .await
        .unwrap();
    let stopping = database
        .request_workspace_action(workspace.id, WorkspaceAction::Stop, admin.user_id, 5)
        .await
        .unwrap();
    let workspace = database
        .record_workspace_observation(stopping.id, WorkspaceObservation::Stopped, admin.user_id, 6)
        .await
        .unwrap();
    let config = AppConfig {
        installation_id: "image-update-api".parse().unwrap(),
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
        workspace,
    )
}

fn request(token: &str, workspace_id: uuid::Uuid, generation: u64, key: &str) -> Request<Body> {
    Request::builder()
        .method(Method::PUT)
        .uri(format!("/api/v1/workspaces/{workspace_id}/image"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Idempotency-Key", key)
        .header("Content-Type", "application/json")
        .body(Body::from(
            json!({"image": NEW_IMAGE, "expected_generation": generation}).to_string(),
        ))
        .unwrap()
}

async fn body(response: axum::response::Response) -> (StatusCode, Vec<u8>) {
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, bytes.to_vec())
}

#[tokio::test]
async fn system_admin_updates_a_stopped_workspace_once_and_replays_the_response() {
    let (app, database, workspace) = app().await;
    let first = app
        .clone()
        .oneshot(request(
            ADMIN_TOKEN,
            workspace.id,
            workspace.generation,
            "image-update-1",
        ))
        .await
        .unwrap();
    let (status, first_body) = body(first).await;
    assert_eq!(status, StatusCode::OK);
    let response: Value = serde_json::from_slice(&first_body).unwrap();
    assert_eq!(response["workspace"]["image"], NEW_IMAGE);
    assert_eq!(response["workspace"]["state"], "stopped");
    assert_eq!(
        response["workspace"]["generation"],
        workspace.generation + 1
    );

    let replay = app
        .oneshot(request(
            ADMIN_TOKEN,
            workspace.id,
            workspace.generation,
            "image-update-1",
        ))
        .await
        .unwrap();
    let (replay_status, replay_body) = body(replay).await;
    assert_eq!(replay_status, status);
    assert_eq!(replay_body, first_body);
    let events = database
        .list_events(workspace.organization_id, Some(workspace.id), 20)
        .await
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == "workspace.image_updated")
            .count(),
        1
    );
}

#[tokio::test]
async fn non_system_admin_cannot_change_a_workspace_image() {
    let (app, _, workspace) = app().await;
    let response = app
        .oneshot(request(
            MEMBER_TOKEN,
            workspace.id,
            workspace.generation,
            "member-image-update",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
