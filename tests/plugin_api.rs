use std::{fs, net::SocketAddr, sync::Arc};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use memeloop_workspace_control::{
    api::{AppState, router},
    auth::Role,
    config::{AppConfig, InstallationId},
    plugins::PluginRuntime,
    storage::{CreateOrganization, Database},
};
use serde_json::{Value, json};
use tower::ServiceExt;

const ADMIN_TOKEN: &str = "plugin-admin-token-000000000000000000000";
const ORG_ADMIN_TOKEN: &str = "plugin-org-admin-token-00000000000000000";

#[tokio::test]
async fn plugin_configuration_is_scoped_versioned_and_removable() {
    let installation_id: InstallationId = "plugin-api".parse().unwrap();
    let database = Database::connect("sqlite::memory:", installation_id.clone())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let admin = database
        .create_user("Admin", ADMIN_TOKEN, true, 1)
        .await
        .unwrap();
    let org_admin = database
        .create_user("Org Admin", ORG_ADMIN_TOKEN, false, 2)
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Plugins".to_owned(),
                owner_user_id: admin.user_id,
            },
            3,
        )
        .await
        .unwrap();
    database
        .upsert_membership(
            organization.id,
            org_admin.user_id,
            Role::OrganizationAdmin,
            4,
        )
        .await
        .unwrap();

    let packages = tempfile::tempdir().unwrap();
    let package = packages.path().join("quota-policy");
    fs::create_dir(&package).unwrap();
    fs::write(
        package.join("plugin.json"),
        json!({
            "id":"quota-policy",
            "name":"Quota policy",
            "version":"1.0.0",
            "description":"A declarative test package",
            "wit_version":"0.1.0",
            "wasm":null,
            "workspace_create_policy":false,
            "denial_codes":[],
            "configuration":{
                "schema":{
                    "type":"object",
                    "additionalProperties":false,
                    "required":["maximum"],
                    "properties":{"maximum":{"type":"integer","minimum":1}}
                },
                "default":{"maximum":10}
            }
        })
        .to_string(),
    )
    .unwrap();
    let plugins = PluginRuntime::load(Some(packages.path()), database.clone()).unwrap();
    let mut state = AppState::new(
        AppConfig {
            installation_id,
            listen_address: SocketAddr::from(([127, 0, 0, 1], 0)),
            database_url: "sqlite::memory:".to_owned(),
            replica_count: 1,
            instance_id: "test".to_owned(),
            ssh_public_host: None,
            internal_ssh_host: None,
            web_shell_public_origin: None,
            prometheus_url: None,
            plugin_dir: None,
        },
        database.clone(),
    );
    state.set_plugin_runtime(plugins);
    let app = router(Arc::new(state));

    let list = app
        .clone()
        .oneshot(
            authenticated(Request::get("/api/v1/plugins"), ORG_ADMIN_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let list = body_json(list).await;
    assert_eq!(list[0]["declared_contributions"], json!(["configuration"]));

    let installation_forbidden = app
        .clone()
        .oneshot(
            authenticated(
                Request::get("/api/v1/plugins/quota-policy/configuration"),
                ORG_ADMIN_TOKEN,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(installation_forbidden.status(), StatusCode::FORBIDDEN);

    let path = format!(
        "/api/v1/plugins/quota-policy/configuration?organization_id={}",
        organization.id
    );
    let put = app
        .clone()
        .oneshot(
            authenticated(Request::put(&path), ORG_ADMIN_TOKEN)
                .header("content-type", "application/json")
                .header("idempotency-key", "configure-org-policy")
                .body(Body::from(
                    json!({"expected_version":0,"value":{"maximum":3}}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);
    let put = body_json(put).await;
    assert_eq!(put["scope"], "organization");
    assert_eq!(put["source"], "organization");
    assert_eq!(put["scope_version"], 1);
    assert_eq!(put["valid"], true);

    let conflict = app
        .clone()
        .oneshot(
            authenticated(Request::put(&path), ORG_ADMIN_TOKEN)
                .header("content-type", "application/json")
                .header("idempotency-key", "stale-org-policy")
                .body(Body::from(
                    json!({"expected_version":0,"value":{"maximum":4}}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);

    let deleted = app
        .oneshot(
            authenticated(Request::delete(&path), ORG_ADMIN_TOKEN)
                .header("content-type", "application/json")
                .header("idempotency-key", "delete-org-policy")
                .body(Body::from(json!({"expected_version":1}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::OK);
    let deleted = body_json(deleted).await;
    assert_eq!(deleted["source"], "default");
    assert_eq!(deleted["scope_version"], 0);

    let audit = database.list_audit(organization.id, 10).await.unwrap();
    assert!(
        audit
            .iter()
            .any(|record| record.action == "plugin.configuration.put")
    );
    assert!(
        audit
            .iter()
            .any(|record| record.action == "plugin.configuration.delete")
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
