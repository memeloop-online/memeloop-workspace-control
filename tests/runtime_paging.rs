use std::{net::SocketAddr, sync::Arc};

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use memeloop_workspace_control::{
    api::{AppState, router},
    auth::Role,
    config::AppConfig,
    crypto::EnvelopeCipher,
    quota::Resources,
    storage::{CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate, Database},
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::AccessMode,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN_TOKEN: &str = "api-admin-000000000000000000000000000000";
const MEMBER_TOKEN: &str = "api-member-000000000000000000000000000000";

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
    let config = AppConfig {
        installation_id: "api-test".parse().unwrap(),
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

async fn seeded_organization(
    database: &Database,
    owner_id: Uuid,
    name: &str,
    now: i64,
) -> (Uuid, Uuid) {
    let organization = database
        .create_organization(
            CreateOrganization {
                name: name.to_owned(),
                owner_user_id: owner_id,
            },
            now,
        )
        .await
        .unwrap();
    database
        .set_organization_quota(
            organization.id,
            Resources {
                cpu_millis: 100_000,
                memory_mib: 100_000,
                gpu_count: 100,
                disk_gib: 100_000,
            },
            now,
        )
        .await
        .unwrap();
    let template = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(organization.id),
                yaml: WorkspaceTemplateDocument::new(
                    format!("{name} template"),
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
            now,
        )
        .await
        .unwrap();
    (organization.id, template.id)
}

async fn seeded_workspace(
    database: &Database,
    organization_id: Uuid,
    owner_id: Uuid,
    template_id: Uuid,
    name: &str,
    now: i64,
) -> Uuid {
    database
        .create_workspace(
            CreateWorkspace {
                organization_id,
                owner_id,
                name: name.to_owned(),
                template_id,
                resources: None,
                organization_injection_refs: None,
                user_injection_refs: None,
            },
            true,
            owner_id,
            now,
        )
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn workspace_runtime_query_ids_are_rejected_before_kubernetes() {
    let (app, _, _) = test_app().await;
    let organization_id = Uuid::nil();

    for (label, uri) in [
        (
            "missing workspace_ids",
            format!("/api/v1/workspace-runtimes?organization_id={organization_id}"),
        ),
        (
            "empty workspace_ids",
            format!("/api/v1/workspace-runtimes?organization_id={organization_id}&workspace_ids="),
        ),
        (
            "invalid workspace_ids",
            format!(
                "/api/v1/workspace-runtimes?organization_id={organization_id}&workspace_ids=not-a-uuid"
            ),
        ),
    ] {
        let response = app
            .clone()
            .oneshot(request(Method::GET, &uri, Some(ADMIN_TOKEN), None, None))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{label} should be rejected without contacting Kubernetes"
        );
    }

    let too_many = (1..=101)
        .map(|value| Uuid::from_u128(value).to_string())
        .collect::<Vec<_>>()
        .join(",");
    let response = app
        .oneshot(request(
            Method::GET,
            &format!(
                "/api/v1/workspace-runtimes?organization_id={organization_id}&workspace_ids={too_many}"
            ),
            Some(ADMIN_TOKEN),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn workspace_runtime_ids_are_scoped_to_the_requested_organization() {
    let (app, database, admin_id) = test_app().await;
    let (own_organization_id, own_template_id) =
        seeded_organization(&database, admin_id, "Runtime own", 10).await;
    let (other_organization_id, other_template_id) =
        seeded_organization(&database, admin_id, "Runtime other", 20).await;
    let other_workspace_id = seeded_workspace(
        &database,
        other_organization_id,
        admin_id,
        other_template_id,
        "foreign",
        20,
    )
    .await;
    let own_workspace_id = seeded_workspace(
        &database,
        own_organization_id,
        admin_id,
        own_template_id,
        "own",
        10,
    )
    .await;

    assert!(
        database
            .list_workspaces_by_ids(own_organization_id, &[other_workspace_id])
            .await
            .unwrap()
            .is_empty()
    );

    let admin_response = app
        .clone()
        .oneshot(request(
            Method::GET,
            &format!(
                "/api/v1/workspace-runtimes?organization_id={own_organization_id}&workspace_ids={other_workspace_id}"
            ),
            Some(ADMIN_TOKEN),
            None,
            None,
        ))
        .await
        .unwrap();
    let (status, response_body) = body(admin_response).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<Value>(&response_body).unwrap(),
        json!([])
    );

    let member = database
        .create_user("Runtime member", MEMBER_TOKEN, false, 30)
        .await
        .unwrap();
    database
        .upsert_membership(own_organization_id, member.user_id, Role::Member, 30)
        .await
        .unwrap();

    let member_response = app
        .clone()
        .oneshot(request(
            Method::GET,
            &format!(
                "/api/v1/workspace-runtimes?organization_id={own_organization_id}&workspace_ids={other_workspace_id}"
            ),
            Some(MEMBER_TOKEN),
            None,
            None,
        ))
        .await
        .unwrap();
    let (status, response_body) = body(member_response).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<Value>(&response_body).unwrap(),
        json!([])
    );

    let cross_organization_query = app
        .oneshot(request(
            Method::GET,
            &format!(
                "/api/v1/workspace-runtimes?organization_id={other_organization_id}&workspace_ids={own_workspace_id}"
            ),
            Some(MEMBER_TOKEN),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(cross_organization_query.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn workspace_page_cursor_and_search_are_keyset_scoped() {
    let (app, database, admin_id) = test_app().await;
    let (organization_id, template_id) =
        seeded_organization(&database, admin_id, "Workspace page", 10).await;
    seeded_workspace(
        &database,
        organization_id,
        admin_id,
        template_id,
        "alpha-first",
        10,
    )
    .await;
    seeded_workspace(
        &database,
        organization_id,
        admin_id,
        template_id,
        "beta",
        20,
    )
    .await;
    seeded_workspace(
        &database,
        organization_id,
        admin_id,
        template_id,
        "alpha-second",
        30,
    )
    .await;

    let first = app
        .clone()
        .oneshot(request(
            Method::GET,
            &format!("/api/v1/workspaces?organization_id={organization_id}&limit=1"),
            Some(ADMIN_TOKEN),
            None,
            None,
        ))
        .await
        .unwrap();
    let (status, first_body) = body(first).await;
    assert_eq!(status, StatusCode::OK);
    let first: Value = serde_json::from_slice(&first_body).unwrap();
    assert_eq!(first["items"].as_array().unwrap().len(), 1);
    assert_eq!(first["items"][0]["workspace"]["name"], "alpha-first");
    let first_cursor = first["next_cursor"].as_str().unwrap();

    let second = app
        .clone()
        .oneshot(request(
            Method::GET,
            &format!(
                "/api/v1/workspaces?organization_id={organization_id}&limit=1&cursor={first_cursor}"
            ),
            Some(ADMIN_TOKEN),
            None,
            None,
        ))
        .await
        .unwrap();
    let (status, second_body) = body(second).await;
    assert_eq!(status, StatusCode::OK);
    let second: Value = serde_json::from_slice(&second_body).unwrap();
    assert_eq!(second["items"][0]["workspace"]["name"], "beta");

    // Applying search before the cursor is important when a cursor came from a broader page.
    let filtered = app
        .clone()
        .oneshot(request(
            Method::GET,
            &format!(
                "/api/v1/workspaces?organization_id={organization_id}&limit=1&search=alpha&cursor={first_cursor}"
            ),
            Some(ADMIN_TOKEN),
            None,
            None,
        ))
        .await
        .unwrap();
    let (status, filtered_body) = body(filtered).await;
    assert_eq!(status, StatusCode::OK);
    let filtered: Value = serde_json::from_slice(&filtered_body).unwrap();
    assert_eq!(filtered["items"][0]["workspace"]["name"], "alpha-second");
    assert!(filtered["next_cursor"].is_null());
}

#[tokio::test]
async fn workspace_page_search_covers_workspace_fields_case_insensitively() {
    let (app, database, admin_id) = test_app().await;
    let (organization_id, default_template_id) =
        seeded_organization(&database, admin_id, "Workspace search", 10).await;
    database
        .upsert_image_policy("registry.example/Search-Image:42", true, 99)
        .await
        .unwrap();

    for (name, now) in [("first", 10), ("second", 20)] {
        seeded_workspace(
            &database,
            organization_id,
            admin_id,
            default_template_id,
            name,
            now,
        )
        .await;
    }

    let mut searchable_spec = WorkspaceTemplateSpec::standard(
        "registry.example/Search-Image:42",
        AccessMode::Internal,
        Resources {
            cpu_millis: 1_000,
            memory_mib: 2_048,
            gpu_count: 0,
            disk_gib: 20,
        },
    );
    searchable_spec.workspace_user = "node-dev".to_owned();
    let searchable_template = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(organization_id),
                yaml: WorkspaceTemplateDocument::new("Search template", searchable_spec)
                    .to_yaml()
                    .unwrap(),
            },
            true,
            30,
        )
        .await
        .unwrap();
    let searchable_workspace_id = seeded_workspace(
        &database,
        organization_id,
        admin_id,
        searchable_template.id,
        "searchable workspace",
        30,
    )
    .await;
    let searchable_workspace = database
        .get_workspace(searchable_workspace_id)
        .await
        .unwrap();

    async fn search_names(app: &Router, organization_id: Uuid, search: &str) -> Vec<String> {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("organization_id", &organization_id.to_string())
            .append_pair("search", search)
            .finish();
        let response = app
            .clone()
            .oneshot(request(
                Method::GET,
                &format!("/api/v1/workspaces?{query}"),
                Some(ADMIN_TOKEN),
                None,
                None,
            ))
            .await
            .unwrap();
        let (status, response_body) = body(response).await;
        assert_eq!(status, StatusCode::OK);
        serde_json::from_slice::<Value>(&response_body).unwrap()["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["workspace"]["name"].as_str().unwrap().to_owned())
            .collect()
    }

    assert_eq!(
        search_names(&app, organization_id, "SEARCHABLE WORKSPACE").await,
        vec!["searchable workspace"]
    );
    assert_eq!(
        search_names(&app, organization_id, "SEARCH-IMAGE:42").await,
        vec!["searchable workspace"]
    );
    assert_eq!(
        search_names(&app, organization_id, "NODE-DEV").await,
        vec!["searchable workspace"]
    );
    assert_eq!(
        search_names(&app, organization_id, "PROVISIONING")
            .await
            .len(),
        3
    );
    assert_eq!(
        search_names(&app, organization_id, &searchable_workspace.short_id).await,
        vec!["searchable workspace"]
    );
    // LIKE metacharacters are treated literally rather than broadening the result set.
    assert!(search_names(&app, organization_id, "%").await.is_empty());
}
