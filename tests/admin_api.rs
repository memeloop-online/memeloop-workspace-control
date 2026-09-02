use std::{
    net::SocketAddr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use memeloop_workspace_control::{
    api::{AppState, router},
    auth::ApiKeyScope,
    config::{AppConfig, InstallationId},
    quota::Resources,
    storage::{CreateOrganization, Database},
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::AccessMode,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN_TOKEN: &str = "admin-api-token-000000000000000000000000000";
const MEMBER_TOKEN: &str = "member-api-token-00000000000000000000000000";
const CREATED_TOKEN: &str = "created-api-token-000000000000000000000000";

#[tokio::test]
async fn management_api_enforces_system_and_organization_boundaries() {
    let initial_key_expiry = test_key_expiry();
    let installation_id: InstallationId = "admin-test".parse().unwrap();
    let database = Database::connect("sqlite::memory:", installation_id.clone())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let admin = database
        .create_user("Admin", ADMIN_TOKEN, true, 100)
        .await
        .unwrap();
    database
        .create_user("Member", MEMBER_TOKEN, false, 101)
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Managed Org".to_owned(),
                owner_user_id: admin.user_id,
            },
            102,
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

    let forbidden = app
        .clone()
        .oneshot(
            authenticated(Request::get("/api/v1/admin/users"), MEMBER_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let created = app
        .clone()
        .oneshot(
            authenticated(Request::post("/api/v1/admin/users"), ADMIN_TOKEN)
                .header("content-type", "application/json")
                .header("idempotency-key", "create-managed-user")
                .body(Body::from(
                    json!({
                        "display_name":"Created User",
                        "token":CREATED_TOKEN,
                        "scopes":[
                            "manage_organization",
                            "create_workspace",
                            "read_workspace",
                            "connect_workspace",
                            "change_workspace_state"
                        ],
                        "expires_at": initial_key_expiry
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let created: Value = body_json(created).await;
    let created_user_id = created["id"].as_str().unwrap();
    assert!(created.get("token").is_none());

    // Explicit grants are stored as named scopes and an expiry, never as a
    // wildcard or unbounded initial key.
    let created_principal = app
        .clone()
        .oneshot(
            authenticated(Request::get("/api/v1/me"), CREATED_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created_principal.status(), StatusCode::OK);
    let created_principal: Value = body_json(created_principal).await;
    assert_eq!(
        created_principal["api_key_scopes"],
        json!([
            "manage_organization",
            "create_workspace",
            "read_workspace",
            "connect_workspace",
            "change_workspace_state"
        ])
    );
    assert_eq!(created_principal["api_key_expires_at"], initial_key_expiry);

    let set_user_quota = app
        .clone()
        .oneshot(
            authenticated(
                Request::put(format!("/api/v1/admin/users/{created_user_id}/quota")),
                ADMIN_TOKEN,
            )
            .header("content-type", "application/json")
            .header("idempotency-key", "set-created-user-quota")
            .body(Body::from(
                json!({"cpu_millis":2000,"memory_mib":4096,"gpu_count":0,"disk_gib":50})
                    .to_string(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(set_user_quota.status(), StatusCode::NO_CONTENT);
    let get_user_quota = app
        .clone()
        .oneshot(
            authenticated(
                Request::get(format!("/api/v1/admin/users/{created_user_id}/quota")),
                CREATED_TOKEN,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(body_json(get_user_quota).await["disk_gib"], 50);

    let membership = app
        .clone()
        .oneshot(
            authenticated(
                Request::put(format!(
                    "/api/v1/organizations/{}/members/{created_user_id}",
                    organization.id
                )),
                ADMIN_TOKEN,
            )
            .header("content-type", "application/json")
            .header("idempotency-key", "grant-created-user")
            .body(Body::from(json!({"role":"organization_admin"}).to_string()))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(membership.status(), StatusCode::NO_CONTENT);

    let quota = json!({"cpu_millis":4000,"memory_mib":8192,"gpu_count":1,"disk_gib":100});
    let set_quota = app
        .clone()
        .oneshot(
            authenticated(
                Request::put(format!("/api/v1/organizations/{}/quota", organization.id)),
                ADMIN_TOKEN,
            )
            .header("content-type", "application/json")
            .header("idempotency-key", "set-managed-quota")
            .body(Body::from(quota.to_string()))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(set_quota.status(), StatusCode::NO_CONTENT);

    let organizations = app
        .clone()
        .oneshot(
            authenticated(Request::get("/api/v1/organizations"), CREATED_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(organizations.status(), StatusCode::OK);
    let organizations: Value = body_json(organizations).await;
    assert_eq!(organizations["items"][0]["id"], organization.id.to_string());

    let image = "registry.example/workspace@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let image_policy = app
        .clone()
        .oneshot(
            authenticated(Request::put("/api/v1/admin/images"), ADMIN_TOKEN)
                .header("content-type", "application/json")
                .header("idempotency-key", "allow-standard-image")
                .body(Body::from(
                    json!({"image":image,"enabled":true}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(image_policy.status(), StatusCode::OK);
    assert_eq!(body_json(image_policy).await["contract_version"], 1);

    let template_spec = WorkspaceTemplateSpec::standard(
        image,
        AccessMode::Internal,
        Resources {
            cpu_millis: 2_000,
            memory_mib: 4_096,
            gpu_count: 0,
            disk_gib: 50,
        },
    );
    let template_yaml = WorkspaceTemplateDocument::new("Standard", template_spec.clone())
        .to_yaml()
        .unwrap();
    let mut legacy_environment_spec = template_spec.clone();
    legacy_environment_spec
        .environment
        .insert("LEGACY_TOKEN".to_owned(), "must-use-injection".to_owned());
    let legacy_environment_yaml =
        WorkspaceTemplateDocument::new("Legacy environment", legacy_environment_spec)
            .to_yaml()
            .unwrap();
    let rejected_environment = app
        .clone()
        .oneshot(
            authenticated(Request::post("/api/v1/templates"), ADMIN_TOKEN)
                .header("content-type", "application/json")
                .header("idempotency-key", "reject-template-environment")
                .body(Body::from(
                    json!({
                        "organization_id": organization.id,
                        "yaml": legacy_environment_yaml.clone()
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected_environment.status(), StatusCode::BAD_REQUEST);

    let template = app
        .clone()
        .oneshot(
            authenticated(Request::post("/api/v1/templates"), ADMIN_TOKEN)
                .header("content-type", "application/json")
                .header("idempotency-key", "create-standard-template")
                .body(Body::from(
                    json!({
                        "organization_id": organization.id,
                        "yaml": template_yaml.clone()
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(template.status(), StatusCode::CREATED);
    let template_id = body_json(template).await["id"].as_str().unwrap().to_owned();

    let rejected_environment_replace = app
        .clone()
        .oneshot(
            authenticated(
                Request::put(format!("/api/v1/templates/{template_id}")),
                ADMIN_TOKEN,
            )
            .header("content-type", "application/json")
            .header("idempotency-key", "reject-template-environment-replace")
            .body(Body::from(
                json!({"yaml": legacy_environment_yaml}).to_string(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        rejected_environment_replace.status(),
        StatusCode::BAD_REQUEST
    );

    let templates = app
        .clone()
        .oneshot(
            authenticated(
                Request::get(format!(
                    "/api/v1/templates?organization_id={}",
                    organization.id
                )),
                CREATED_TOKEN,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(templates.status(), StatusCode::OK);
    assert_eq!(body_json(templates).await[0]["name"], "Standard");

    let organization_admin_update = app
        .clone()
        .oneshot(
            authenticated(
                Request::put(format!("/api/v1/templates/{template_id}")),
                CREATED_TOKEN,
            )
            .header("content-type", "application/json")
            .header("idempotency-key", "organization-admin-template-update")
            .body(Body::from(json!({"yaml": template_yaml}).to_string()))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(organization_admin_update.status(), StatusCode::OK);

    let mut privileged_spec = template_spec;
    privileged_spec.cluster_access = true;
    let privileged_yaml = WorkspaceTemplateDocument::new("Standard", privileged_spec)
        .to_yaml()
        .unwrap();
    let forbidden_privilege_escalation = app
        .clone()
        .oneshot(
            authenticated(
                Request::put(format!("/api/v1/templates/{template_id}")),
                CREATED_TOKEN,
            )
            .header("content-type", "application/json")
            .header("idempotency-key", "organization-admin-cluster-access")
            .body(Body::from(json!({"yaml": privileged_yaml}).to_string()))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        forbidden_privilege_escalation.status(),
        StatusCode::FORBIDDEN
    );

    let disabled = app
        .clone()
        .oneshot(
            authenticated(
                Request::put(format!("/api/v1/templates/{template_id}/enabled")),
                ADMIN_TOKEN,
            )
            .header("content-type", "application/json")
            .header("idempotency-key", "disable-standard-template")
            .body(Body::from(json!({"enabled": false}).to_string()))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(disabled.status(), StatusCode::OK);
    assert_eq!(body_json(disabled).await["enabled"], false);

    let deleted_template = app
        .clone()
        .oneshot(
            authenticated(
                Request::delete(format!("/api/v1/templates/{template_id}")),
                CREATED_TOKEN,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted_template.status(), StatusCode::NO_CONTENT);

    let templates_after_delete = app
        .clone()
        .oneshot(
            authenticated(
                Request::get(format!(
                    "/api/v1/templates?organization_id={}",
                    organization.id
                )),
                CREATED_TOKEN,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(body_json(templates_after_delete).await, json!([]));

    let revoked = app
        .clone()
        .oneshot(
            authenticated(
                Request::delete(format!(
                    "/api/v1/organizations/{}/members/{created_user_id}",
                    organization.id
                )),
                ADMIN_TOKEN,
            )
            .header("idempotency-key", "revoke-created-user")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);
    let organizations = app
        .clone()
        .oneshot(
            authenticated(Request::get("/api/v1/organizations"), CREATED_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        body_json(organizations).await,
        json!({"items": [], "next_cursor": null})
    );

    let audit = app
        .clone()
        .oneshot(
            authenticated(
                Request::get(format!("/api/v1/audit?organization_id={}", organization.id)),
                ADMIN_TOKEN,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(audit.status(), StatusCode::OK);
    let audit: Value = body_json(audit).await;
    assert!(audit["items"].as_array().unwrap().iter().all(|record| {
        (record["actor_user_id"] == admin.user_id.to_string()
            && record["actor_display_name"] == "Admin")
            || (record["actor_user_id"] == created_user_id
                && record["actor_display_name"] == "Created User")
    }));

    let scaling = app
        .oneshot(
            authenticated(Request::get("/api/v1/admin/scaling"), ADMIN_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(scaling.status(), StatusCode::OK);
    let scaling: Value = body_json(scaling).await;
    assert_eq!(scaling["database_mode"], "sqlite");
    assert_eq!(scaling["configured_replicas"], 1);
    assert_eq!(scaling["schema_version"], 16);
}

#[tokio::test]
async fn admin_user_initial_key_policy_rejects_escalation_and_invalid_expiry() {
    const SCOPED_ADMIN_TOKEN: &str = "scoped-admin-api-token-000000000000000000000";
    const LEGACY_ADMIN_TOKEN: &str = "legacy-admin-api-token-000000000000000000000";
    const NEXT_TOKEN: &str = "next-user-api-token-000000000000000000000000000";
    let initial_key_now = test_unix_timestamp();
    let initial_key_expiry = initial_key_now + 30 * 24 * 60 * 60;
    let installation_id: InstallationId = "initial-key-policy".parse().unwrap();
    let database = Database::connect("sqlite::memory:", installation_id.clone())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
        .create_user_with_initial_key(
            "Scoped administrator",
            SCOPED_ADMIN_TOKEN,
            true,
            vec![ApiKeyScope::ManageSystem],
            initial_key_expiry,
            initial_key_now,
        )
        .await
        .unwrap();
    database
        .create_user("Legacy administrator", LEGACY_ADMIN_TOKEN, true, 101)
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

    let compatibility = app
        .clone()
        .oneshot(
            authenticated(Request::post("/api/v1/admin/users"), LEGACY_ADMIN_TOKEN)
                .header("content-type", "application/json")
                .header("idempotency-key", "initial-key-compatible")
                .body(Body::from(
                    json!({"display_name": "Compatible user", "token": NEXT_TOKEN}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(compatibility.status(), StatusCode::CREATED);
    let compatibility_principal = app
        .clone()
        .oneshot(
            authenticated(Request::get("/api/v1/me"), NEXT_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let compatibility_principal: Value = body_json(compatibility_principal).await;
    assert_eq!(
        compatibility_principal["api_key_scopes"],
        json!([
            "manage_api_keys",
            "create_workspace",
            "read_workspace",
            "connect_workspace",
            "change_workspace_state"
        ])
    );
    assert!(
        compatibility_principal["api_key_expires_at"]
            .as_i64()
            .is_some()
    );

    let escalation = app
        .clone()
        .oneshot(
            authenticated(Request::post("/api/v1/admin/users"), SCOPED_ADMIN_TOKEN)
                .header("content-type", "application/json")
                .header("idempotency-key", "initial-key-escalation")
                .body(Body::from(
                    json!({
                        "display_name": "Escalated user",
                        "token": NEXT_TOKEN,
                        "scopes": ["manage_system", "read_workspace"],
                        "expires_at": 1_900_000_000,
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(escalation.status(), StatusCode::FORBIDDEN);

    let expired = app
        .clone()
        .oneshot(
            authenticated(Request::post("/api/v1/admin/users"), SCOPED_ADMIN_TOKEN)
                .header("content-type", "application/json")
                .header("idempotency-key", "initial-key-expired")
                .body(Body::from(
                    json!({
                        "display_name": "Expired user",
                        "token": NEXT_TOKEN,
                        "scopes": ["manage_system"],
                        "expires_at": 1,
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(expired.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn creating_a_user_with_an_organization_membership_is_atomic_and_authorized() {
    const RESTRICTED_ADMIN_TOKEN: &str = "restricted-admin-api-token-0000000000000000000";
    const JOINED_TOKEN: &str = "joined-user-api-token-00000000000000000000000000";
    const MISSING_ORG_TOKEN: &str = "missing-org-user-api-token-000000000000000000000";
    const FORBIDDEN_TOKEN: &str = "forbidden-org-user-api-token-0000000000000000000";
    let now = test_unix_timestamp();
    let expiry = now + 30 * 24 * 60 * 60;
    let installation_id: InstallationId = "atomic-onboard".parse().unwrap();
    let database = Database::connect("sqlite::memory:", installation_id.clone())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let administrator = database
        .create_user("Administrator", ADMIN_TOKEN, true, now)
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Atomic onboarding".to_owned(),
                owner_user_id: administrator.user_id,
            },
            now,
        )
        .await
        .unwrap();
    database
        .create_user_with_initial_key(
            "Restricted administrator",
            RESTRICTED_ADMIN_TOKEN,
            true,
            vec![ApiKeyScope::ManageSystem],
            expiry,
            now,
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

    let joined = app
        .clone()
        .oneshot(
            authenticated(Request::post("/api/v1/admin/users"), ADMIN_TOKEN)
                .header("content-type", "application/json")
                .header("idempotency-key", "atomic-user-joined")
                .body(Body::from(
                    json!({
                        "display_name": "Joined user",
                        "token": JOINED_TOKEN,
                        "organization_id": organization.id,
                        "organization_role": "member"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(joined.status(), StatusCode::CREATED);
    let joined: Value = body_json(joined).await;
    let joined_user_id = joined["id"].as_str().unwrap();
    let members = app
        .clone()
        .oneshot(
            authenticated(
                Request::get(format!(
                    "/api/v1/organizations/{}/members?search=Joined%20user",
                    organization.id
                )),
                ADMIN_TOKEN,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(members.status(), StatusCode::OK);
    let members: Value = body_json(members).await;
    assert_eq!(members["items"][0]["user"]["id"], joined_user_id);
    assert_eq!(members["items"][0]["role"], "member");

    let nonexistent = app
        .clone()
        .oneshot(
            authenticated(Request::post("/api/v1/admin/users"), ADMIN_TOKEN)
                .header("content-type", "application/json")
                .header("idempotency-key", "atomic-user-missing-org")
                .body(Body::from(
                    json!({
                        "display_name": "Missing organization user",
                        "token": MISSING_ORG_TOKEN,
                        "organization_id": Uuid::now_v7(),
                        "organization_role": "member"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(nonexistent.status(), StatusCode::NOT_FOUND);

    let forbidden = app
        .clone()
        .oneshot(
            authenticated(Request::post("/api/v1/admin/users"), RESTRICTED_ADMIN_TOKEN)
                .header("content-type", "application/json")
                .header("idempotency-key", "atomic-user-no-membership-scope")
                .body(Body::from(
                    json!({
                        "display_name": "Forbidden organization user",
                        "token": FORBIDDEN_TOKEN,
                        "organization_id": organization.id,
                        "organization_role": "member"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    for (token, name) in [
        (MISSING_ORG_TOKEN, "Missing organization user"),
        (FORBIDDEN_TOKEN, "Forbidden organization user"),
    ] {
        let authenticated_user = app
            .clone()
            .oneshot(
                authenticated(Request::get("/api/v1/me"), token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(authenticated_user.status(), StatusCode::UNAUTHORIZED);
        let listed = app
            .clone()
            .oneshot(
                authenticated(
                    Request::get(format!(
                        "/api/v1/admin/users?search={}",
                        name.replace(' ', "%20")
                    )),
                    ADMIN_TOKEN,
                )
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            body_json(listed).await["items"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}

fn test_key_expiry() -> i64 {
    test_unix_timestamp() + 30 * 24 * 60 * 60
}

fn test_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[tokio::test]
async fn user_and_organization_management_are_paginated_and_safe() {
    let installation_id: InstallationId = "admin-page-test".parse().unwrap();
    let database = Database::connect("sqlite::memory:", installation_id.clone())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let admin = database
        .create_user("Admin", ADMIN_TOKEN, true, 1)
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Original".to_owned(),
                owner_user_id: admin.user_id,
            },
            2,
        )
        .await
        .unwrap();
    let storage = database.clone();
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

    storage
        .create_user(
            "Admin Two",
            "admin-two-api-token-0000000000000000000000000",
            false,
            3,
        )
        .await
        .unwrap();

    let page = app
        .clone()
        .oneshot(
            authenticated(
                Request::get("/api/v1/admin/users?limit=1&search=adm"),
                ADMIN_TOKEN,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let page = body_json(page).await;
    assert_eq!(page["items"][0]["id"], admin.user_id.to_string());
    // Preserve the historical response shape unless an organization is requested.
    assert!(page["items"][0].get("membership_role").is_none());
    let cursor = page["next_cursor"].as_str().unwrap();
    let second_page = app
        .clone()
        .oneshot(
            authenticated(
                Request::get(format!(
                    "/api/v1/admin/users?limit=1&search=adm&cursor={cursor}"
                )),
                ADMIN_TOKEN,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second_page.status(), StatusCode::OK);
    assert_eq!(
        body_json(second_page).await["items"][0]["display_name"],
        "Admin Two"
    );

    let organization_roles = app
        .clone()
        .oneshot(
            authenticated(
                Request::get(format!(
                    "/api/v1/admin/users?organization_id={}&limit=1&search=adm",
                    organization.id
                )),
                ADMIN_TOKEN,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(organization_roles.status(), StatusCode::OK);
    let organization_roles = body_json(organization_roles).await;
    assert_eq!(
        organization_roles["items"][0]["membership_role"],
        "organization_admin"
    );
    let role_cursor = organization_roles["next_cursor"].as_str().unwrap();
    let no_membership = app
        .clone()
        .oneshot(
            authenticated(
                Request::get(format!(
                    "/api/v1/admin/users?organization_id={}&limit=1&search=adm&cursor={role_cursor}",
                    organization.id
                )),
                ADMIN_TOKEN,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_membership.status(), StatusCode::OK);
    assert!(body_json(no_membership).await["items"][0]["membership_role"].is_null());

    let self_lockout = app
        .clone()
        .oneshot(
            authenticated(
                Request::put(format!("/api/v1/admin/users/{}", admin.user_id)),
                ADMIN_TOKEN,
            )
            .header("content-type", "application/json")
            .body(Body::from(json!({"disabled": true}).to_string()))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(self_lockout.status(), StatusCode::BAD_REQUEST);

    assert!(matches!(
        storage
            .update_user(admin.user_id, None, Some(false), None)
            .await,
        Err(memeloop_workspace_control::storage::StorageError::LastSystemAdmin)
    ));

    let rename = app
        .clone()
        .oneshot(
            authenticated(
                Request::put(format!("/api/v1/organizations/{}", organization.id)),
                ADMIN_TOKEN,
            )
            .header("content-type", "application/json")
            .body(Body::from(json!({"name":"Renamed"}).to_string()))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rename.status(), StatusCode::OK);
    assert_eq!(body_json(rename).await["name"], "Renamed");
    let members = app
        .clone()
        .oneshot(
            authenticated(
                Request::get(format!(
                    "/api/v1/organizations/{}/members?limit=1",
                    organization.id
                )),
                ADMIN_TOKEN,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(members.status(), StatusCode::OK);
    assert_eq!(
        body_json(members).await["items"][0]["user"]["id"],
        admin.user_id.to_string()
    );
}

#[tokio::test]
async fn user_page_search_treats_like_metacharacters_literally() {
    let installation_id: InstallationId = "admin-search-test".parse().unwrap();
    let database = Database::connect("sqlite::memory:", installation_id.clone())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
        .create_user("Search admin", ADMIN_TOKEN, true, 1)
        .await
        .unwrap();
    for (display_name, token) in [
        (
            "Percent % user",
            "literal-percent-token-000000000000000000000",
        ),
        (
            "Underscore _ user",
            "literal-underscore-token-000000000000000000",
        ),
        (
            "Backslash \\ user",
            "literal-backslash-token-0000000000000000000",
        ),
    ] {
        database
            .create_user(display_name, token, false, 2)
            .await
            .unwrap();
    }
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

    for (search, expected) in [
        ("%", "Percent % user"),
        ("_", "Underscore _ user"),
        ("\\", "Backslash \\ user"),
    ] {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("search", search)
            .finish();
        let response = app
            .clone()
            .oneshot(
                authenticated(
                    Request::get(format!("/api/v1/admin/users?{query}")),
                    ADMIN_TOKEN,
                )
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let users = body_json(response).await["items"]
            .as_array()
            .unwrap()
            .to_vec();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0]["display_name"], expected);
    }
}

fn authenticated(
    builder: axum::http::request::Builder,
    token: &str,
) -> axum::http::request::Builder {
    builder.header("authorization", format!("Bearer {token}"))
}

async fn body_json(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
