use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use memeloop_workspace_control::{
    api::{AppState, router},
    auth::ApiKeyScope,
    config::AppConfig,
    quota::Resources,
    storage::{CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate, Database},
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::{AccessMode, Workspace, WorkspaceAction, WorkspaceObservation},
};
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tower::ServiceExt;
const ADMIN_TOKEN: &str = "settings-admin-000000000000000000000000000";
const MEMBER_TOKEN: &str = "settings-member-0000000000000000000000000";
const OLD_IMAGE: &str = "registry.example/workspace@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
async fn app(
    ready: bool,
) -> (
    Router,
    Database,
    memeloop_workspace_control::workspaces::Workspace,
) {
    let key_now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let database = Database::connect("sqlite::memory:", "settings-api".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
        .upsert_image_policy(OLD_IMAGE, true, 1)
        .await
        .unwrap();
    let admin = database
        .create_user_with_initial_key(
            "Admin",
            ADMIN_TOKEN,
            true,
            memeloop_workspace_control::auth::ApiKeyScope::initial_key_defaults(true),
            key_now + 24 * 60 * 60,
            key_now,
        )
        .await
        .unwrap();
    database
        .create_user_with_initial_key(
            "Member",
            MEMBER_TOKEN,
            false,
            memeloop_workspace_control::auth::ApiKeyScope::initial_key_defaults(false),
            key_now + 24 * 60 * 60,
            key_now,
        )
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "CPU overcommit".to_owned(),
                owner_user_id: admin.user_id,
            },
            2,
        )
        .await
        .unwrap();
    let mut document = WorkspaceTemplateDocument::new(
        "CPU overcommit",
        WorkspaceTemplateSpec::standard(
            OLD_IMAGE,
            AccessMode::Internal,
            Resources {
                cpu_millis: 10_000,
                memory_mib: 2_048,
                gpu_count: 0,
                disk_gib: 20,
            },
        ),
    );
    document.spec.cpu_overcommit_ratio = Some(10);
    document.spec.pod_requests.cpu_millis = 2_000;
    document.spec.buildkit = true;
    let yaml = document.to_yaml().unwrap();
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
                name: "Stopped CPU overcommit".to_owned(),
                template_id: template.id,
                resources: None,
                temporary_storage_gib: None,
                use_longhorn: false,
                organization_injection_refs: None,
                user_injection_refs: None,
            },
            true,
            admin.user_id,
            4,
        )
        .await
        .unwrap();
    let workspace = if ready {
        database
            .record_workspace_observation(
                workspace.id,
                workspace.generation,
                WorkspaceObservation::Ready,
                admin.user_id,
                5,
            )
            .await
            .unwrap()
            .unwrap()
    } else {
        let stopping = database
            .request_workspace_action(workspace.id, WorkspaceAction::Stop, admin.user_id, 5)
            .await
            .unwrap();
        database
            .record_workspace_observation(
                stopping.id,
                stopping.generation,
                WorkspaceObservation::Stopped,
                admin.user_id,
                6,
            )
            .await
            .unwrap()
            .unwrap()
    };
    let config = AppConfig {
        installation_id: "settings-api".parse().unwrap(),
        listen_address: SocketAddr::from(([127, 0, 0, 1], 0)),
        database_url: "sqlite::memory:".to_owned(),
        replica_count: 1,
        instance_id: "test".to_owned(),
        ssh_public_host: None,
        internal_ssh_host: None,
        web_shell_public_origin: Some("https://shell.example.test".to_owned()),
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
fn settings(workspace: &Workspace, name: &str, cpu: u64) -> Value {
    json!({"name":name,"cpu_millis":cpu,"memory_mib":workspace.template.resources.memory_mib,"gpu_count":workspace.template.resources.gpu_count,"expected_generation":workspace.generation})
}

fn request(token: &str, workspace: &Workspace, key: &str, input: &Value) -> Request<Body> {
    Request::builder()
        .method(Method::PUT)
        .uri(format!("/api/v1/workspaces/{}/settings", workspace.id))
        .header("Authorization", format!("Bearer {token}"))
        .header("Idempotency-Key", key)
        .header("Content-Type", "application/json")
        .body(Body::from(input.to_string()))
        .unwrap()
}

async fn body(response: axum::response::Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn job_count(database: &Database, workspace: &Workspace) -> i64 {
    let Database::Sqlite { pool, .. } = database else {
        panic!("SQLite fixture")
    };
    sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE workspace_id=?1")
        .bind(workspace.id.to_string())
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn ready_rename_does_not_change_generation_or_enqueue_a_rollout() {
    let (app, database, workspace) = app(true).await;
    let jobs = job_count(&database, &workspace).await;
    let input = settings(
        &workspace,
        &"开发".repeat(60),
        workspace.template.resources.cpu_millis,
    );
    let (status, response) = body(
        app.oneshot(request(ADMIN_TOKEN, &workspace, "rename-ready", &input))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["workspace"]["name"], input["name"]);
    assert_eq!(response["workspace"]["generation"], workspace.generation);
    assert_eq!(response["workspace"]["state"], "ready");
    assert_eq!(job_count(&database, &workspace).await, jobs);
    let updated = database.get_workspace(workspace.id).await.unwrap();
    assert_eq!(updated.template, workspace.template);
    assert_eq!(updated.home_volume_binding, workspace.home_volume_binding);
}

#[tokio::test]
async fn running_compute_and_stale_generation_are_conflicts_without_mutation() {
    let (app, database, workspace) = app(true).await;
    let before_jobs = job_count(&database, &workspace).await;
    let compute = settings(
        &workspace,
        "must not persist",
        workspace.template.resources.cpu_millis + 1000,
    );
    let mut stale = settings(
        &workspace,
        "stale name",
        workspace.template.resources.cpu_millis,
    );
    stale["expected_generation"] = json!(workspace.generation + 1);
    for (key, input) in [("running-compute", compute), ("stale-settings", stale)] {
        let (status, response) = body(
            app.clone()
                .oneshot(request(ADMIN_TOKEN, &workspace, key, &input))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{response}");
        assert_eq!(
            response["error"]["code"],
            "workspace_settings_update_conflict"
        );
    }
    assert_eq!(
        database.get_workspace(workspace.id).await.unwrap(),
        workspace
    );
    assert_eq!(job_count(&database, &workspace).await, before_jobs);
}

#[tokio::test]
async fn stopped_compute_update_is_atomic_and_idempotently_replayed() {
    let (app, database, workspace) = app(false).await;
    let input = settings(&workspace, "支付开发", 12000);
    let jobs = job_count(&database, &workspace).await;
    let first = body(
        app.clone()
            .oneshot(request(ADMIN_TOKEN, &workspace, "stopped-settings", &input))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(first.0, StatusCode::OK, "{}", first.1);
    let replay = body(
        app.oneshot(request(ADMIN_TOKEN, &workspace, "stopped-settings", &input))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(replay, first);
    assert_eq!(first.1["workspace"]["generation"], workspace.generation + 1);
    assert_eq!(first.1["workspace"]["state"], "stopped");
    assert_eq!(job_count(&database, &workspace).await, jobs + 1);
    let updated = database.get_workspace(workspace.id).await.unwrap();
    assert_eq!(updated.template.resources.cpu_millis, 12000);
    assert_eq!(
        updated.template.resources.disk_gib,
        workspace.template.resources.disk_gib
    );
    assert_eq!(updated.home_volume_binding, workspace.home_volume_binding);
    assert_eq!(
        updated.scratch_volume_binding,
        workspace.scratch_volume_binding
    );
    assert_eq!(updated.node_pool, workspace.node_pool);
}

#[tokio::test]
async fn organization_scope_and_template_restrictions_apply_before_mutation() {
    let (app, database, workspace) = app(false).await;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let restricted = database
        .create_api_key(
            workspace.owner_id,
            "Restricted settings",
            vec![ApiKeyScope::ChangeWorkspaceState],
            Some(now + 3600),
            Some(vec![]),
            now,
        )
        .await
        .unwrap();
    let read_only = database
        .create_api_key(
            workspace.owner_id,
            "Read only settings",
            vec![ApiKeyScope::ReadWorkspace],
            Some(now + 3600),
            None,
            now,
        )
        .await
        .unwrap();
    let input = settings(&workspace, "must not persist", 12000);
    for (key, token) in [
        ("wrong-organization", MEMBER_TOKEN),
        ("restricted-template", restricted.token.as_str()),
        ("read-only", read_only.token.as_str()),
    ] {
        let (status, response) = body(
            app.clone()
                .oneshot(request(token, &workspace, key, &input))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{key}: {response}");
    }
    assert_eq!(
        database.get_workspace(workspace.id).await.unwrap(),
        workspace
    );
}

#[tokio::test]
async fn a_state_change_key_can_rename_but_cannot_obtain_connection_details() {
    let (app, database, workspace) = app(true).await;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let key = database
        .create_api_key(
            workspace.owner_id,
            "State changes only",
            vec![ApiKeyScope::ChangeWorkspaceState],
            Some(now + 3600),
            None,
            now,
        )
        .await
        .unwrap();
    let input = settings(
        &workspace,
        "Rename without connect scope",
        workspace.template.resources.cpu_millis,
    );
    let (status, response) = body(
        app.oneshot(request(&key.token, &workspace, "rename-no-connect", &input))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["workspace"]["name"], input["name"]);
    for field in [
        "web_shell_url",
        "ssh_connection",
        "ssh_host",
        "ssh_port",
        "ssh_command",
        "ssh_config",
    ] {
        assert!(
            response[field].is_null(),
            "{field} must respect connect_workspace scope"
        );
    }
}
