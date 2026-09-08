use std::{net::SocketAddr, sync::Arc};

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode},
};
use memeloop_workspace_control::{
    api::{AppState, router},
    auth::Role,
    config::{AppConfig, InstallationId},
    storage::{CreateOrganization, Database},
};
use serde_json::json;
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN_TOKEN: &str = "membership-idempotency-admin-token-000000000";

struct TestContext {
    app: Router,
    database: Database,
    organization_id: Uuid,
    owner_id: Uuid,
    second_user_id: Uuid,
}

async fn test_context() -> TestContext {
    let installation_id: InstallationId = "membership-idem".parse().unwrap();
    let database = Database::connect("sqlite::memory:", installation_id.clone())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let owner = database
        .create_user("Owner", ADMIN_TOKEN, true, 1)
        .await
        .unwrap();
    let second_user = database
        .create_user(
            "Second user",
            "membership-idempotency-second-token-000000",
            false,
            2,
        )
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Membership idempotency organization".to_owned(),
                owner_user_id: owner.user_id,
            },
            3,
        )
        .await
        .unwrap();
    let app = router(Arc::new(AppState::new(
        AppConfig {
            installation_id,
            listen_address: SocketAddr::from(([127, 0, 0, 1], 0)),
            database_url: "sqlite::memory:".to_owned(),
            replica_count: 1,
            instance_id: "membership-idempotency-test".to_owned(),
            ssh_public_host: None,
            internal_ssh_host: None,
            web_shell_public_origin: None,
            port_mapping_public_domain: None,
            prometheus_url: None,
            plugin_dir: None,
        },
        database.clone(),
    )));
    TestContext {
        app,
        database,
        organization_id: organization.id,
        owner_id: owner.user_id,
        second_user_id: second_user.user_id,
    }
}

fn membership_request(method: Method, uri: String, key: &str, role: Option<Role>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {ADMIN_TOKEN}"))
        .header("idempotency-key", key);
    if let Some(role) = role {
        builder = builder.header("content-type", "application/json");
        builder
            .body(Body::from(json!({"role": role}).to_string()))
            .unwrap()
    } else {
        builder.body(Body::empty()).unwrap()
    }
}

#[tokio::test]
async fn membership_storage_errors_release_idempotency_reservations() {
    let context = test_context().await;
    let member_path = format!(
        "/api/v1/organizations/{}/members/{}",
        context.organization_id, context.owner_id
    );

    // Demoting the only organization administrator fails. The same request
    // must be retryable after the administrator invariant is repaired; it
    // must not remain stuck behind the failed reservation.
    let failed_upsert = context
        .app
        .clone()
        .oneshot(membership_request(
            Method::PUT,
            member_path.clone(),
            "failed-membership-upsert",
            Some(Role::Member),
        ))
        .await
        .unwrap();
    assert_eq!(failed_upsert.status(), StatusCode::CONFLICT);

    context
        .database
        .upsert_membership(
            context.organization_id,
            context.second_user_id,
            Role::OrganizationAdmin,
            4,
        )
        .await
        .unwrap();
    let retried_upsert = context
        .app
        .clone()
        .oneshot(membership_request(
            Method::PUT,
            member_path,
            "failed-membership-upsert",
            Some(Role::Member),
        ))
        .await
        .unwrap();
    assert_eq!(retried_upsert.status(), StatusCode::NO_CONTENT);

    let second_path = format!(
        "/api/v1/organizations/{}/members/{}",
        context.organization_id, context.second_user_id
    );
    // Removing the remaining administrator follows the same failure path.
    let failed_remove = context
        .app
        .clone()
        .oneshot(membership_request(
            Method::DELETE,
            second_path.clone(),
            "failed-membership-remove",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(failed_remove.status(), StatusCode::CONFLICT);

    context
        .database
        .upsert_membership(
            context.organization_id,
            context.owner_id,
            Role::OrganizationAdmin,
            5,
        )
        .await
        .unwrap();
    let retried_remove = context
        .app
        .oneshot(membership_request(
            Method::DELETE,
            second_path,
            "failed-membership-remove",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(retried_remove.status(), StatusCode::NO_CONTENT);
}
