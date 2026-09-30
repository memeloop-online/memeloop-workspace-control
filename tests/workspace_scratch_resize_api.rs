use std::{net::SocketAddr, sync::Arc, time::SystemTime};

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
    workspaces::{AccessMode, Workspace, WorkspaceAction, WorkspaceObservation},
};
use serde_json::{Value, json};
use tower::ServiceExt;

const ADMIN_TOKEN: &str = "scratch-resize-admin-000000000000000000000000";
const MEMBER_TOKEN: &str = "scratch-resize-member-00000000000000000000000";
const IMAGE: &str = "registry.example/workspace@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

async fn app() -> (Router, Database, Workspace) {
    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let database = Database::connect("sqlite::memory:", "scratch-resize-api".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database.upsert_image_policy(IMAGE, true, now).await.unwrap();
    let admin = database
        .create_user_with_initial_key(
            "Admin",
            ADMIN_TOKEN,
            true,
            memeloop_workspace_control::auth::ApiKeyScope::initial_key_defaults(true),
            now + 86_400,
            now,
        )
        .await
        .unwrap();
    database
        .create_user_with_initial_key(
            "Member",
            MEMBER_TOKEN,
            false,
            memeloop_workspace_control::auth::ApiKeyScope::initial_key_defaults(false),
            now + 86_400,
            now,
        )
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Scratch resize".to_owned(),
                owner_user_id: admin.user_id,
            },
            now + 1,
        )
        .await
        .unwrap();
    let yaml = WorkspaceTemplateDocument::new(
        "Scratch resize",
        WorkspaceTemplateSpec::standard(
            IMAGE,
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
            now + 2,
        )
        .await
        .unwrap();
    let workspace = database
        .create_workspace(
            CreateWorkspace {
                organization_id: organization.id,
                owner_id: admin.user_id,
                name: "Scratch resize target".to_owned(),
                template_id: template.id,
                resources: None,
                organization_injection_refs: None,
                user_injection_refs: None,
            },
            true,
            admin.user_id,
            now + 3,
        )
        .await
        .unwrap();
    let stopping = database
        .request_workspace_action(workspace.id, WorkspaceAction::Stop, admin.user_id, now + 4)
        .await
        .unwrap();
    let workspace = database
        .record_workspace_observation(
            stopping.id,
            WorkspaceObservation::Stopped,
            admin.user_id,
            now + 5,
        )
        .await
        .unwrap();
    let config = AppConfig {
        installation_id: "scratch-resize-api".parse().unwrap(),
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

fn request(
    token: &str,
    workspace_id: uuid::Uuid,
    generation: u64,
    temporary_storage_gib: u64,
    key: &str,
) -> Request<Body> {
    Request::builder()
        .method(Method::PUT)
        .uri(format!(
            "/api/v1/workspaces/{workspace_id}/temporary-storage"
        ))
        .header("Authorization", format!("Bearer {token}"))
        .header("Idempotency-Key", key)
        .header("Content-Type", "application/json")
        .body(Body::from(
            json!({
                "temporary_storage_gib": temporary_storage_gib,
                "expected_generation": generation
            })
            .to_string(),
        ))
        .unwrap()
}

async fn body(response: axum::response::Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn resizes_stopped_workspace_and_records_snapshot_accounting_job_and_audit() {
    let (app, database, workspace) = app().await;
    let response = app
        .clone()
        .oneshot(request(
            ADMIN_TOKEN,
            workspace.id,
            workspace.generation,
            5,
            "scratch-resize-1",
        ))
        .await
        .unwrap();
    let (status, response) = body(response).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["workspace"]["state"], "stopped");
    assert_eq!(
        response["workspace"]["generation"],
        workspace.generation + 1
    );
    assert_eq!(
        response["workspace"]["storage_policy"]["temporary_storage_gib"],
        5
    );
    let replay = app
        .oneshot(request(
            ADMIN_TOKEN,
            workspace.id,
            workspace.generation,
            5,
            "scratch-resize-1",
        ))
        .await
        .unwrap();
    let (replay_status, replay_body) = body(replay).await;
    assert_eq!(replay_status, status);
    assert_eq!(replay_body, response);

    let Database::Sqlite {
        pool,
        installation_id,
    } = &database
    else {
        unreachable!();
    };
    let row = sqlx::query("SELECT temporary_storage_gib, template_snapshot_yaml FROM workspaces WHERE installation_id = ?1 AND id = ?2")
        .bind(installation_id.as_str())
        .bind(workspace.id.to_string())
        .fetch_one(pool)
        .await
        .unwrap();
    use sqlx::Row;
    assert_eq!(row.try_get::<i64, _>("temporary_storage_gib").unwrap(), 5);
    let snapshot: String = row.try_get("template_snapshot_yaml").unwrap();
    let document = WorkspaceTemplateDocument::parse(&snapshot).unwrap();
    assert_eq!(document.spec.storage_policy.temporary_storage_gib, 5);

    let job_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE workspace_id = ?1 AND kind = 'reconcile_workspace' AND payload_json LIKE '%temporary_storage_updated%'")
        .bind(workspace.id.to_string())
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(job_count, 1);
    let audit = database
        .page_audit(memeloop_workspace_control::storage::AuditFilter {
            organization_id: Some(workspace.organization_id),
            action: Some("workspace.temporary_storage_updated".to_owned()),
            limit: 10,
            offset: 0,
            actor: None,
            workspace: Some(workspace.id.to_string()),
            query: None,
        })
        .await
        .unwrap();
    assert_eq!(audit.items.len(), 1);
}

#[tokio::test]
async fn rejects_unauthorized_out_of_range_stale_and_running_updates() {
    let (app, _database, workspace) = app().await;
    for (token, generation, value, key, expected) in [
        (
            MEMBER_TOKEN,
            workspace.generation,
            5,
            "scratch-denied",
            StatusCode::FORBIDDEN,
        ),
        (
            ADMIN_TOKEN,
            workspace.generation,
            0,
            "scratch-invalid",
            StatusCode::BAD_REQUEST,
        ),
        (
            ADMIN_TOKEN,
            workspace.generation + 1,
            5,
            "scratch-stale",
            StatusCode::CONFLICT,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(request(token, workspace.id, generation, value, key))
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    let starting = _database
        .request_workspace_action(
            workspace.id,
            WorkspaceAction::Start,
            workspace.owner_id,
            100,
        )
        .await
        .unwrap();
    let response = app
        .oneshot(request(
            ADMIN_TOKEN,
            workspace.id,
            starting.generation,
            5,
            "scratch-running",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}
