use std::{net::SocketAddr, sync::Arc};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use memeloop_workspace_control::{
    api::{AppState, router},
    config::{AppConfig, InstallationId},
    quota::Resources,
    storage::{CreateOrganization, Database},
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::AccessMode,
};
use serde_json::{Value, json};
use tower::ServiceExt;

const ADMIN_TOKEN: &str = "admin-api-token-000000000000000000000000000";
const MEMBER_TOKEN: &str = "member-api-token-00000000000000000000000000";
const CREATED_TOKEN: &str = "created-api-token-000000000000000000000000";

#[tokio::test]
async fn management_api_enforces_system_and_organization_boundaries() {
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
                    json!({"display_name":"Created User","token":CREATED_TOKEN}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let created: Value = body_json(created).await;
    let created_user_id = created["id"].as_str().unwrap();
    assert!(created.get("token").is_none());

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
    assert_eq!(scaling["schema_version"], 15);
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

fn authenticated(
    builder: axum::http::request::Builder,
    token: &str,
) -> axum::http::request::Builder {
    builder.header("authorization", format!("Bearer {token}"))
}

async fn body_json(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
