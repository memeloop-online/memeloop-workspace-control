use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use http_body_util::BodyExt;
use memeloop_workspace_control::{
    api::{AppState, internal_router},
    config::{AppConfig, InstallationId},
    crypto::EnvelopeCipher,
    injections::{InjectionItem, InjectionKind, InjectionScope, InjectionValue},
    quota::Resources,
    storage::{
        CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate, Database, InjectionScopeRef,
    },
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::{AccessMode, WorkspaceObservation},
};
use tower::ServiceExt;

const USER_TOKEN: &str = "ssh-user-token-000000000000000000000000000000";
const INTERNAL_TOKEN: &str = "ssh-internal-token-00000000000000000000000000";

#[tokio::test]
async fn authorized_keys_command_returns_only_restricted_workspace_target() {
    let installation_id = "ssh-test".parse::<InstallationId>().unwrap();
    let database = Database::connect("sqlite::memory:", installation_id.clone())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
        .upsert_image_policy("registry.example/workspace:1", true, 99)
        .await
        .unwrap();
    let user = database
        .create_user("SSH User", USER_TOKEN, false, 100)
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "SSH Org".to_owned(),
                owner_user_id: user.user_id,
            },
            101,
        )
        .await
        .unwrap();
    let template = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(organization.id),
                yaml: WorkspaceTemplateDocument::new(
                    "Public SSH",
                    WorkspaceTemplateSpec::standard(
                        "registry.example/workspace:1",
                        AccessMode::Public,
                        Resources {
                            cpu_millis: 500,
                            memory_mib: 512,
                            gpu_count: 0,
                            disk_gib: 5,
                        },
                    ),
                )
                .to_yaml()
                .unwrap(),
            },
            false,
            101,
        )
        .await
        .unwrap();
    let workspace = database
        .create_workspace(
            CreateWorkspace {
                organization_id: organization.id,
                owner_id: user.user_id,
                name: "ssh".to_owned(),
                template_id: template.id,
                organization_injection_refs: None,
                user_injection_refs: None,
            },
            false,
            user.user_id,
            102,
        )
        .await
        .unwrap();
    database
        .record_workspace_observation(workspace.id, WorkspaceObservation::Ready, user.user_id, 103)
        .await
        .unwrap();
    let cipher = EnvelopeCipher::from_base64(&STANDARD.encode([9_u8; 32])).unwrap();
    database
        .replace_injection(
            &cipher,
            InjectionScopeRef {
                scope: InjectionScope::User,
                scope_id: user.user_id,
            },
            ssh_key("ssh-ed25519 AQIDBA== laptop"),
            user.user_id,
            104,
        )
        .await
        .unwrap();

    let mut state = AppState::with_cipher(
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
        database.clone(),
        cipher.clone(),
    );
    state.set_internal_auth_token(INTERNAL_TOKEN);
    let app = internal_router(Arc::new(state));
    let uri = format!(
        "/api/v1/internal/ssh/authorized-key?login=access%2B{}&key_type=ssh-ed25519&key_base64=AQIDBA%3D%3D",
        workspace.short_id
    );

    let response = app
        .clone()
        .oneshot(internal_request(&uri, INTERNAL_TOKEN))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let line = String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(line.starts_with("restrict,port-forwarding,permitopen=\""));
    assert!(line.contains(&format!(
        "workspace.ws-ssh-test-{}.svc.cluster.local:2222",
        workspace.short_id
    )));
    assert!(line.contains("ssh-ed25519 AQIDBA=="));

    let unauthenticated = app
        .clone()
        .oneshot(internal_request(
            &uri,
            "wrong-wrong-wrong-wrong-wrong-wrong",
        ))
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

    database
        .replace_injection(
            &cipher,
            InjectionScopeRef {
                scope: InjectionScope::User,
                scope_id: user.user_id,
            },
            ssh_key("ssh-ed25519 BQYHCA== replacement"),
            user.user_id,
            105,
        )
        .await
        .unwrap();
    let revoked = app
        .oneshot(internal_request(&uri, INTERNAL_TOKEN))
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);
}

fn ssh_key(value: &str) -> InjectionItem {
    InjectionItem {
        key: "primary-ssh-key".to_owned(),
        kind: InjectionKind::SshPublicKey,
        target: "authorized_keys".to_owned(),
        value: InjectionValue::Utf8(value.to_owned()),
        sensitive: false,
        locked: false,
        version: 0,
        file_mode: None,
        owner: None,
        group: None,
        template_selector: None,
        labels: BTreeMap::new(),
    }
}

fn internal_request(uri: &str, token: &str) -> Request<Body> {
    Request::get(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}
