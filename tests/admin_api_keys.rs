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
    auth::{ApiKeyScope, Role},
    config::{AppConfig, InstallationId},
    storage::{ApiKeyListStatus, AuditFilter, CreateOrganization, Database},
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN_TOKEN: &str = "admin-api-key-list-token-0000000000000000000";
const TARGET_TOKEN: &str = "target-api-key-list-token-000000000000000000";
const SYSTEM_WITHOUT_KEY_SCOPE_TOKEN: &str = "admin-without-key-scope-token-00000000000000";
const SYSTEM_WITHOUT_SYSTEM_SCOPE_TOKEN: &str = "admin-without-system-scope-token-000000000000";
const ORGANIZATION_ADMIN_TOKEN: &str = "organization-admin-key-token-0000000000000000";

#[tokio::test]
async fn system_admin_can_list_and_idempotently_revoke_a_users_api_keys() {
    let installation_id: InstallationId = "admin-api-keys-test".parse().unwrap();
    let database = Database::connect("sqlite::memory:", installation_id.clone())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let admin = database
        .create_user("Administrator", ADMIN_TOKEN, true, 100)
        .await
        .unwrap();
    let target = database
        .create_user("Target user", TARGET_TOKEN, false, 101)
        .await
        .unwrap();
    let created = database
        .create_api_key(
            target.user_id,
            "Secondary target key",
            vec![ApiKeyScope::ReadWorkspace],
            Some(unix_timestamp() + 30 * 24 * 60 * 60),
            unix_timestamp(),
        )
        .await
        .unwrap();
    let initial_key_id = database
        .list_api_keys(target.user_id)
        .await
        .unwrap()
        .into_iter()
        .find(|key| key.id != created.summary.id)
        .unwrap()
        .id;
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
    let endpoint = format!("/api/v1/admin/users/{}/api-keys", target.user_id);

    let organization_admin_style_access = app
        .clone()
        .oneshot(
            authenticated(Request::get(&endpoint), TARGET_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        organization_admin_style_access.status(),
        StatusCode::FORBIDDEN
    );

    let first_page = app
        .clone()
        .oneshot(
            authenticated(
                Request::get(format!("{endpoint}?status=all&limit=1")),
                ADMIN_TOKEN,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first_page.status(), StatusCode::OK);
    let first_page = body_json(first_page).await;
    let first_key = &first_page["items"][0];
    assert!(first_key.get("token").is_none());
    assert!(first_key.get("token_hash").is_none());
    let cursor = first_page["next_cursor"].as_str().unwrap();

    let second_page = app
        .clone()
        .oneshot(
            authenticated(
                Request::get(format!("{endpoint}?status=all&limit=1&cursor={cursor}")),
                ADMIN_TOKEN,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second_page.status(), StatusCode::OK);
    assert_eq!(
        body_json(second_page).await["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let invalid_reason = app
        .clone()
        .oneshot(
            authenticated(
                Request::delete(format!("{endpoint}/{}", created.summary.id)),
                ADMIN_TOKEN,
            )
            .header("content-type", "application/json")
            .body(Body::from(json!({"reason":"  "}).to_string()))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid_reason.status(), StatusCode::BAD_REQUEST);

    let missing_reason = app
        .clone()
        .oneshot(
            authenticated(
                Request::delete(format!("{endpoint}/{}", created.summary.id)),
                ADMIN_TOKEN,
            )
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_reason.status(), StatusCode::BAD_REQUEST);

    for _ in 0..2 {
        let revoked = app
            .clone()
            .oneshot(
                authenticated(
                    Request::delete(format!("{endpoint}/{}", created.summary.id)),
                    ADMIN_TOKEN,
                )
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"reason":"suspected credential exposure"}).to_string(),
                ))
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoked.status(), StatusCode::NO_CONTENT);
    }

    let revoked = app
        .clone()
        .oneshot(
            authenticated(
                Request::get(format!("{endpoint}?status=revoked")),
                ADMIN_TOKEN,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let revoked = body_json(revoked).await;
    assert_eq!(revoked["items"].as_array().unwrap().len(), 1);
    assert_eq!(revoked["items"][0]["id"], created.summary.id.to_string());

    let audit = app
        .clone()
        .oneshot(
            authenticated(
                Request::get("/api/v1/audit?action=user.api_key.admin_revoke"),
                ADMIN_TOKEN,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(audit.status(), StatusCode::OK);
    let audit = body_json(audit).await;
    let records = audit["items"].as_array().unwrap();
    assert_eq!(records.len(), 1);
    let metadata = records[0]["metadata"].as_object().unwrap();
    assert_eq!(metadata.len(), 3);
    assert_eq!(metadata["target_user_id"], target.user_id.to_string());
    assert_eq!(metadata["api_key_id"], created.summary.id.to_string());
    assert_eq!(metadata["reason"], "suspected credential exposure");
    assert_eq!(records[0]["actor_user_id"], admin.user_id.to_string());

    let revoked_last_key = app
        .clone()
        .oneshot(
            authenticated(
                Request::delete(format!("{endpoint}/{initial_key_id}")),
                ADMIN_TOKEN,
            )
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"reason":"administrator forced final-key revocation"}).to_string(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoked_last_key.status(), StatusCode::NO_CONTENT);
    let target_authentication = app
        .oneshot(
            authenticated(Request::get("/api/v1/me"), TARGET_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(target_authentication.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn target_key_administration_requires_both_system_scopes_and_noops_are_silent() {
    let now = unix_timestamp();
    let expiry = now + 30 * 24 * 60 * 60;
    let installation_id: InstallationId = "admin-key-security".parse().unwrap();
    let database = Database::connect("sqlite::memory:", installation_id.clone())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let authorized = database
        .create_user_with_initial_key(
            "Authorized administrator",
            ADMIN_TOKEN,
            true,
            vec![ApiKeyScope::ManageSystem, ApiKeyScope::ManageApiKeys],
            expiry,
            now,
        )
        .await
        .unwrap();
    database
        .create_user_with_initial_key(
            "Administrator without key scope",
            SYSTEM_WITHOUT_KEY_SCOPE_TOKEN,
            true,
            vec![ApiKeyScope::ManageSystem],
            expiry,
            now,
        )
        .await
        .unwrap();
    database
        .create_user_with_initial_key(
            "Administrator without system scope",
            SYSTEM_WITHOUT_SYSTEM_SCOPE_TOKEN,
            true,
            vec![ApiKeyScope::ManageApiKeys],
            expiry,
            now,
        )
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Security test organization".to_owned(),
                owner_user_id: authorized.user_id,
            },
            now,
        )
        .await
        .unwrap();
    let organization_admin = database
        .create_user_with_initial_key(
            "Organization administrator",
            ORGANIZATION_ADMIN_TOKEN,
            false,
            vec![ApiKeyScope::ManageSystem, ApiKeyScope::ManageApiKeys],
            expiry,
            now,
        )
        .await
        .unwrap();
    database
        .upsert_membership(
            organization.id,
            organization_admin.user_id,
            Role::OrganizationAdmin,
            now,
        )
        .await
        .unwrap();
    let target = database
        .create_user("Target user", TARGET_TOKEN, false, now)
        .await
        .unwrap();
    let target_key = database
        .create_api_key(
            target.user_id,
            "Target secondary key",
            vec![ApiKeyScope::ReadWorkspace],
            Some(expiry),
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
        database.clone(),
    )));
    let list_path = format!("/api/v1/admin/users/{}/api-keys", target.user_id);
    let delete_path = format!("{list_path}/{}", target_key.summary.id);

    for token in [
        ORGANIZATION_ADMIN_TOKEN,
        SYSTEM_WITHOUT_KEY_SCOPE_TOKEN,
        SYSTEM_WITHOUT_SYSTEM_SCOPE_TOKEN,
    ] {
        let listed = app
            .clone()
            .oneshot(
                authenticated(Request::get(&list_path), token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(listed.status(), StatusCode::FORBIDDEN, "token {token}");

        let revoked = app
            .clone()
            .oneshot(
                authenticated(Request::delete(&delete_path), token)
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"reason":"not authorized"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoked.status(), StatusCode::FORBIDDEN, "token {token}");
    }

    let audit_before = admin_revoke_audit_count(&database).await;
    for (wrong_target, wrong_key) in [
        (Uuid::now_v7(), target_key.summary.id),
        (target.user_id, Uuid::now_v7()),
    ] {
        let response = app
            .clone()
            .oneshot(
                authenticated(
                    Request::delete(format!(
                        "/api/v1/admin/users/{wrong_target}/api-keys/{wrong_key}"
                    )),
                    ADMIN_TOKEN,
                )
                .header("content-type", "application/json")
                .body(Body::from(json!({"reason":"target cleanup"}).to_string()))
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }
    assert_eq!(admin_revoke_audit_count(&database).await, audit_before);

    let administrator_key_id = database.list_api_keys(authorized.user_id).await.unwrap()[0].id;
    let self_admin_revoke = app
        .clone()
        .oneshot(
            authenticated(
                Request::delete(format!(
                    "/api/v1/admin/users/{}/api-keys/{administrator_key_id}",
                    authorized.user_id
                )),
                ADMIN_TOKEN,
            )
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"reason":"must not bypass recovery protection"}).to_string(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(self_admin_revoke.status(), StatusCode::BAD_REQUEST);
    assert_eq!(admin_revoke_audit_count(&database).await, audit_before);

    let openapi = app
        .oneshot(
            Request::get("/api/v1/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(openapi.status(), StatusCode::OK);
    let paths = &body_json(openapi).await["paths"];
    assert!(
        paths
            .get("/api/v1/admin/users/{user_id}/api-keys")
            .is_some()
    );
    assert!(
        paths
            .get("/api/v1/admin/users/{user_id}/api-keys/{key_id}")
            .is_some()
    );
}

#[tokio::test]
async fn postgres_admin_revocation_is_idempotent_for_a_disabled_target() {
    let Ok(database_url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        eprintln!("skipping PostgreSQL API-key test: MWC_TEST_POSTGRES_URL is not set");
        return;
    };
    let suffix = &Uuid::now_v7().simple().to_string()[..8];
    let installation_id: InstallationId = format!("key-pg-{suffix}").parse().unwrap();
    let database = Database::connect(&database_url, installation_id)
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let now = unix_timestamp();
    let admin = database
        .create_user(
            "PostgreSQL administrator",
            &format!("pg-admin-token-{suffix}-000000000000000000000000"),
            true,
            now,
        )
        .await
        .unwrap();
    let target = database
        .create_user(
            "Disabled PostgreSQL target",
            &format!("pg-target-token-{suffix}-00000000000000000000000"),
            false,
            now,
        )
        .await
        .unwrap();
    let key_id = database.list_api_keys(target.user_id).await.unwrap()[0].id;
    database
        .update_user(target.user_id, None, None, Some(true))
        .await
        .unwrap();

    let first = database
        .admin_revoke_api_key(
            admin.user_id,
            target.user_id,
            key_id,
            "disabled account cleanup",
            now + 1,
        )
        .await
        .unwrap();
    let repeated = database
        .admin_revoke_api_key(
            admin.user_id,
            target.user_id,
            key_id,
            "repeated cleanup",
            now + 2,
        )
        .await
        .unwrap();

    assert!(first.changed);
    assert!(!repeated.changed);
    let revoked_page = database
        .list_api_keys_page(target.user_id, ApiKeyListStatus::Revoked, Some(10), None)
        .await
        .unwrap();
    assert_eq!(revoked_page.items.len(), 1);
    assert_eq!(revoked_page.items[0].id, key_id);
    assert_eq!(admin_revoke_audit_count(&database).await, 1);
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

fn unix_timestamp() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap()
}

async fn admin_revoke_audit_count(database: &Database) -> usize {
    database
        .page_audit(AuditFilter {
            organization_id: None,
            limit: 100,
            offset: 0,
            action: Some("user.api_key.admin_revoke".to_owned()),
            actor: None,
            workspace: None,
            query: None,
        })
        .await
        .unwrap()
        .items
        .len()
}
