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
    crypto::EnvelopeCipher,
    injections::InjectionScope,
    quota::Resources,
    storage::{CreateWorkspaceTemplate, Database, InjectionScopeRef},
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::{AccessMode, WorkspaceObservation},
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN_TOKEN: &str = "api-admin-000000000000000000000000000000";
const OUTSIDER_TOKEN: &str = "api-outsider-0000000000000000000000000000";

async fn test_app() -> (Router, Database, Uuid) {
    let installation_id = "api-test".parse().unwrap();
    let database = Database::connect("sqlite::memory:", installation_id)
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
        .upsert_image_policy("registry.example/workspace:1", true, 99)
        .await
        .unwrap();
    let admin = database
        .create_user("API Admin", ADMIN_TOKEN, true, 1)
        .await
        .unwrap();
    database
        .create_user("Outsider", OUTSIDER_TOKEN, false, 1)
        .await
        .unwrap();
    let config = AppConfig {
        installation_id: "api-test".parse().unwrap(),
        listen_address: SocketAddr::from(([127, 0, 0, 1], 0)),
        database_url: "sqlite::memory:".to_owned(),
        replica_count: 1,
        instance_id: "test".to_owned(),
        ssh_public_host: None,
        internal_ssh_host: None,
        web_shell_public_origin: None,
        prometheus_url: None,
        plugin_dir: None,
    };
    (
        router(Arc::new(AppState::with_cipher(
            config,
            database.clone(),
            EnvelopeCipher::from_base64("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap(),
        ))),
        database,
        admin.user_id,
    )
}

fn request(
    method: Method,
    uri: &str,
    token: Option<&str>,
    idempotency_key: Option<&str>,
    body: Option<Value>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header("Authorization", format!("Bearer {token}"));
    }
    if let Some(key) = idempotency_key {
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

async fn body(response: axum::response::Response) -> (StatusCode, Vec<u8>) {
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, bytes.to_vec())
}

#[tokio::test]
async fn authenticated_workspace_api_enforces_rbac_and_exact_idempotent_replay() {
    let (app, database, admin_id) = test_app().await;
    let unauthenticated = app
        .clone()
        .oneshot(request(Method::GET, "/api/v1/me", None, None, None))
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

    let organization_request = json!({
        "name": "API Organization",
        "owner_user_id": Uuid::nil()
    });
    let first_org = app
        .clone()
        .oneshot(request(
            Method::POST,
            "/api/v1/organizations",
            Some(ADMIN_TOKEN),
            Some("org-request-1"),
            Some(organization_request.clone()),
        ))
        .await
        .unwrap();
    let (first_org_status, first_org_body) = body(first_org).await;
    assert_eq!(first_org_status, StatusCode::CREATED);
    let organization: Value = serde_json::from_slice(&first_org_body).unwrap();
    let organization_id = Uuid::parse_str(organization["id"].as_str().unwrap()).unwrap();

    let replay_org = app
        .clone()
        .oneshot(request(
            Method::POST,
            "/api/v1/organizations",
            Some(ADMIN_TOKEN),
            Some("org-request-1"),
            Some(organization_request),
        ))
        .await
        .unwrap();
    let (replay_org_status, replay_org_body) = body(replay_org).await;
    assert_eq!(replay_org_status, first_org_status);
    assert_eq!(replay_org_body, first_org_body);

    database
        .set_organization_quota(
            organization_id,
            Resources {
                cpu_millis: 2_000,
                memory_mib: 4_096,
                gpu_count: 0,
                disk_gib: 50,
            },
            2,
        )
        .await
        .unwrap();
    let template = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(organization_id),
                yaml: WorkspaceTemplateDocument::new(
                    "API template",
                    WorkspaceTemplateSpec::standard(
                        "registry.example/workspace:1",
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
                .unwrap(),
            },
            true,
            2,
        )
        .await
        .unwrap();

    let workspace_request = json!({
        "organization_id": organization_id,
        "owner_id": admin_id,
        "name": "primary",
        "template_id": template.id,
        "inline_workspace_injections": [{
            "key": "multiline-config",
            "kind": "config_file",
            "target": "/home/workspace/.config/example.yml",
            "value": {"encoding": "utf8", "value": "first:\n\n  nested: true\n"},
            "sensitive": false,
            "locked": false,
            "version": 0,
            "file_mode": 384,
            "owner": "workspace",
            "group": "workspace",
            "template_selector": null,
            "labels": {}
        }]
    });
    let mut unknown_ref_request = workspace_request.clone();
    unknown_ref_request["name"] = json!("unknown-ref");
    unknown_ref_request["organization_injection_refs"] = json!(["missing-key"]);
    let unknown_ref = app
        .clone()
        .oneshot(request(
            Method::POST,
            "/api/v1/workspaces",
            Some(ADMIN_TOKEN),
            Some("unknown-ref-request"),
            Some(unknown_ref_request),
        ))
        .await
        .unwrap();
    assert_eq!(unknown_ref.status(), StatusCode::BAD_REQUEST);

    let mut invalid_workspace_request = workspace_request.clone();
    invalid_workspace_request["inline_workspace_injections"][0]["value"] =
        json!({"encoding": "base64", "value": "not base64!"});
    let invalid = app
        .clone()
        .oneshot(request(
            Method::POST,
            "/api/v1/workspaces",
            Some(ADMIN_TOKEN),
            Some("workspace-request-1"),
            Some(invalid_workspace_request),
        ))
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let first_workspace = app
        .clone()
        .oneshot(request(
            Method::POST,
            "/api/v1/workspaces",
            Some(ADMIN_TOKEN),
            Some("workspace-request-1"),
            Some(workspace_request.clone()),
        ))
        .await
        .unwrap();
    let (first_workspace_status, first_workspace_body) = body(first_workspace).await;
    assert_eq!(
        first_workspace_status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&first_workspace_body)
    );
    let workspace_response: Value = serde_json::from_slice(&first_workspace_body).unwrap();
    let workspace_id =
        Uuid::parse_str(workspace_response["workspace"]["id"].as_str().unwrap()).unwrap();
    assert_eq!(
        workspace_response["namespace"],
        format!(
            "ws-api-test-{}",
            workspace_response["workspace"]["short_id"]
                .as_str()
                .unwrap()
        )
    );
    let summaries = database
        .list_injection_summaries(InjectionScopeRef {
            scope: InjectionScope::Workspace,
            scope_id: workspace_id,
        })
        .await
        .unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].key, "multiline-config");

    let wait_semantics_conflict = app
        .clone()
        .oneshot(request(
            Method::POST,
            "/api/v1/workspaces?wait_until=ready&timeout=1",
            Some(ADMIN_TOKEN),
            Some("workspace-request-1"),
            Some(workspace_request.clone()),
        ))
        .await
        .unwrap();
    assert_eq!(wait_semantics_conflict.status(), StatusCode::CONFLICT);

    let replay_workspace = app
        .clone()
        .oneshot(request(
            Method::POST,
            "/api/v1/workspaces",
            Some(ADMIN_TOKEN),
            Some("workspace-request-1"),
            Some(workspace_request),
        ))
        .await
        .unwrap();
    let (replay_status, replay_body) = body(replay_workspace).await;
    assert_eq!(replay_status, first_workspace_status);
    assert_eq!(replay_body, first_workspace_body);

    let conflict = app
        .clone()
        .oneshot(request(
            Method::POST,
            "/api/v1/workspaces",
            Some(ADMIN_TOKEN),
            Some("workspace-request-1"),
            Some(json!({
                "organization_id": organization_id,
                "owner_id": admin_id,
                "name": "different",
                "template_id": template.id
            })),
        ))
        .await
        .unwrap();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);

    let outsider = app
        .clone()
        .oneshot(request(
            Method::GET,
            &format!("/api/v1/workspaces?organization_id={organization_id}"),
            Some(OUTSIDER_TOKEN),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(outsider.status(), StatusCode::FORBIDDEN);

    database
        .record_workspace_observation(workspace_id, WorkspaceObservation::Ready, admin_id, 3)
        .await
        .unwrap();
    let ready = app
        .clone()
        .oneshot(request(
            Method::GET,
            &format!("/api/v1/workspaces/{workspace_id}"),
            Some(ADMIN_TOKEN),
            None,
            None,
        ))
        .await
        .unwrap();
    let (ready_status, ready_body) = body(ready).await;
    assert_eq!(ready_status, StatusCode::OK);
    let ready: Value = serde_json::from_slice(&ready_body).unwrap();
    let alias = format!("mwc-{}", ready["workspace"]["short_id"].as_str().unwrap());
    assert_eq!(ready["ssh_connection"]["display_name"], "primary");
    assert_eq!(ready["ssh_connection"]["alias"], alias);
    assert_eq!(ready["ssh_connection"]["port"], 2_222);
    assert_eq!(ready["ssh_connection"]["user"], "workspace");
    assert_eq!(ready["ssh_connection"]["app"]["hostname"], alias);
    assert_eq!(
        ready["ssh_connection"]["app"]["port_strategy"],
        "ssh_config"
    );
    assert!(ready["ssh_connection"]["app"]["ssh_port"].is_null());
    assert_eq!(ready["ssh_command"], ready["ssh_connection"]["command"]);
    assert_eq!(ready["ssh_config"], ready["ssh_connection"]["config"]);
    let stop = app
        .oneshot(request(
            Method::POST,
            &format!("/api/v1/workspaces/{workspace_id}/actions/stop"),
            Some(ADMIN_TOKEN),
            Some("stop-request-1"),
            None,
        ))
        .await
        .unwrap();
    let (stop_status, stop_body) = body(stop).await;
    assert_eq!(stop_status, StatusCode::ACCEPTED);
    let stopped: Value = serde_json::from_slice(&stop_body).unwrap();
    assert_eq!(stopped["workspace"]["state"], "stopping");
}
