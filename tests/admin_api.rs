use std::{net::SocketAddr, sync::Arc};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use memeloop_workspace_control::{
    api::{AppState, router},
    config::{AppConfig, InstallationId},
    storage::{CreateOrganization, Database},
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
            .body(Body::from(json!({"role":"member"}).to_string()))
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
    assert_eq!(organizations[0]["id"], organization.id.to_string());

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

    let template = app
        .clone()
        .oneshot(
            authenticated(Request::post("/api/v1/templates"), ADMIN_TOKEN)
                .header("content-type", "application/json")
                .header("idempotency-key", "create-standard-template")
                .body(Body::from(
                    json!({
                        "organization_id": organization.id,
                        "name": "Standard",
                        "image": image,
                        "access_mode": "internal",
                        "resources": {"cpu_millis":2000,"memory_mib":4096,"gpu_count":0,"disk_gib":50}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(template.status(), StatusCode::CREATED);
    let template_id = body_json(template).await["id"].as_str().unwrap().to_owned();

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

    let disabled = app
        .clone()
        .oneshot(
            authenticated(
                Request::put(format!("/api/v1/templates/{template_id}")),
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
    assert_eq!(body_json(organizations).await, json!([]));

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
    assert!(audit.as_array().unwrap().iter().all(|record| {
        record["actor_user_id"] == admin.user_id.to_string()
            && record["actor_display_name"] == "Admin"
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
    assert_eq!(scaling["schema_version"], 9);
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
