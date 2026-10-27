use std::{
    net::SocketAddr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use memeloop_workspace_control::{
    api::{AppState, router},
    config::AppConfig,
    kubernetes::{InternetEgressConfig, ResourceBuilder},
    quota::Resources,
    storage::{CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate, Database},
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::{AccessMode, WorkspaceAction, WorkspaceObservation},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tower::ServiceExt;

const ADMIN_TOKEN: &str = "cpu-overcommit-admin-000000000000000000000000";
const MEMBER_TOKEN: &str = "cpu-overcommit-member-000000000000000000000";
const OLD_IMAGE: &str = "registry.example/workspace@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

async fn app() -> (
    Router,
    Database,
    memeloop_workspace_control::workspaces::Workspace,
) {
    let key_now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let database = Database::connect("sqlite::memory:", "cpu-overcommit-api".parse().unwrap())
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
    let stopping = database
        .request_workspace_action(workspace.id, WorkspaceAction::Stop, admin.user_id, 5)
        .await
        .unwrap();
    let workspace = database
        .record_workspace_observation(
            stopping.id,
            stopping.generation,
            WorkspaceObservation::Stopped,
            admin.user_id,
            6,
        )
        .await
        .unwrap()
        .unwrap();
    let config = AppConfig {
        installation_id: "cpu-overcommit-api".parse().unwrap(),
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
        router(Arc::new(AppState::new(config, database.clone()))),
        database,
        workspace,
    )
}

fn request(token: &str, workspace_id: uuid::Uuid, generation: u64, key: &str) -> Request<Body> {
    Request::builder()
        .method(Method::PUT)
        .uri(format!("/api/v1/workspaces/{workspace_id}/cpu-overcommit"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Idempotency-Key", key)
        .header("Content-Type", "application/json")
        .body(Body::from(
            json!({"cpu_overcommit_ratio": 20, "expected_generation": generation}).to_string(),
        ))
        .unwrap()
}

async fn body(response: axum::response::Response) -> (StatusCode, Vec<u8>) {
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, bytes.to_vec())
}

#[tokio::test]
async fn system_admin_updates_a_stopped_workspace_once_and_replays_the_response() {
    let (app, database, workspace) = app().await;
    let first = app
        .clone()
        .oneshot(request(
            ADMIN_TOKEN,
            workspace.id,
            workspace.generation,
            "cpu-overcommit-1",
        ))
        .await
        .unwrap();
    let (status, first_body) = body(first).await;
    assert_eq!(status, StatusCode::OK);
    let response: Value = serde_json::from_slice(&first_body).unwrap();
    assert_eq!(response["workspace"]["cpu_overcommit_ratio"], 20);
    assert_eq!(response["workspace"]["state"], "stopped");
    assert_eq!(
        response["workspace"]["generation"],
        workspace.generation + 1
    );

    let updated = database.get_workspace(workspace.id).await.unwrap();
    let mut expected_template = workspace.template.clone();
    expected_template.cpu_overcommit_ratio = Some(20);
    assert_eq!(updated.template, expected_template);
    assert_eq!(updated.id, workspace.id);
    assert_eq!(updated.node_pool, workspace.node_pool);
    assert_eq!(updated.runtime, workspace.runtime);
    assert_eq!(updated.home_volume_binding, workspace.home_volume_binding);
    assert_eq!(
        updated.scratch_volume_binding,
        workspace.scratch_volume_binding
    );
    assert_eq!(updated.template.resources.cpu_millis, 10_000);
    assert_eq!(updated.template.cpu_request_millis(2_000), 100);
    assert_eq!(updated.template.cpu_request_millis(250), 13);
    assert_eq!(updated.template.cpu_request_millis(10), 1);

    let before_pod = builder()
        .build(&workspace)
        .unwrap()
        .stateful_set
        .spec
        .unwrap()
        .template
        .spec
        .unwrap();
    let after_pod = builder()
        .build(&updated)
        .unwrap()
        .stateful_set
        .spec
        .unwrap()
        .template
        .spec
        .unwrap();
    let mut request_total = 0;
    for (before, after) in before_pod
        .containers
        .iter()
        .chain(before_pod.init_containers.as_ref().unwrap())
        .zip(
            after_pod
                .containers
                .iter()
                .chain(after_pod.init_containers.as_ref().unwrap()),
        )
    {
        assert_eq!(before.name, after.name);
        let before_resources = before.resources.as_ref().unwrap();
        let mut after_resources = after.resources.clone().unwrap();
        assert_eq!(
            after_resources.limits, before_resources.limits,
            "{} CPU/memory limits changed",
            after.name
        );
        let cpu = &after_resources.requests.as_ref().unwrap()["cpu"].0;
        if after_pod
            .containers
            .iter()
            .any(|container| container.name == after.name)
        {
            request_total += cpu.strip_suffix('m').unwrap().parse::<u64>().unwrap();
        }
        after_resources.requests.as_mut().unwrap().insert(
            "cpu".to_owned(),
            before_resources.requests.as_ref().unwrap()["cpu"].clone(),
        );
        assert_eq!(
            &after_resources, before_resources,
            "{} non-CPU resources changed",
            after.name
        );
    }
    assert_eq!(request_total, 114);
    let Database::Sqlite {
        pool,
        installation_id,
    } = &database
    else {
        unreachable!()
    };
    let audit: String = sqlx::query_scalar("SELECT metadata_json FROM audit_log WHERE installation_id = ?1 AND workspace_id = ?2 AND action = 'workspace.cpu_overcommit_updated'")
        .bind(installation_id.as_str()).bind(workspace.id.to_string()).fetch_one(pool).await.unwrap();
    let audit: Value = serde_json::from_str(&audit).unwrap();
    assert_eq!(audit["previous_cpu_overcommit_ratio"], 10);
    assert_eq!(audit["cpu_overcommit_ratio"], 20);
    let snapshot_yaml: String = sqlx::query_scalar(
        "SELECT template_snapshot_yaml FROM workspaces WHERE installation_id = ?1 AND id = ?2",
    )
    .bind(installation_id.as_str())
    .bind(workspace.id.to_string())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(
        WorkspaceTemplateDocument::parse(&snapshot_yaml)
            .unwrap()
            .spec,
        expected_template
    );

    let replay = app
        .clone()
        .oneshot(request(
            ADMIN_TOKEN,
            workspace.id,
            workspace.generation,
            "cpu-overcommit-1",
        ))
        .await
        .unwrap();
    let (replay_status, replay_body) = body(replay).await;
    assert_eq!(replay_status, status);
    assert_eq!(replay_body, first_body);
    let events = database
        .list_events(workspace.organization_id, Some(workspace.id), 20)
        .await
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.payload["action"] == "cpu_overcommit_updated")
            .count(),
        1
    );
    let starting = database
        .request_workspace_action(
            workspace.id,
            WorkspaceAction::Start,
            workspace.owner_id,
            updated.updated_at + 1,
        )
        .await
        .unwrap();
    assert_eq!(starting.template, expected_template);
    let workload = builder()
        .build(&starting)
        .unwrap()
        .stateful_set
        .spec
        .unwrap();
    assert_eq!(workload.replicas, Some(1));
    assert_eq!(workload.template.spec.unwrap(), after_pod);
}

#[tokio::test]
async fn non_system_admin_cannot_change_workspace_cpu_overcommit() {
    let (app, _, workspace) = app().await;
    let response = app
        .oneshot(request(
            MEMBER_TOKEN,
            workspace.id,
            workspace.generation,
            "member-cpu-overcommit",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn rejects_unauthenticated_stale_running_and_out_of_range_requests_without_mutation() {
    let (app, database, workspace) = app().await;
    let response = app
        .clone()
        .oneshot(request(
            "invalid",
            workspace.id,
            workspace.generation,
            "unauth",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = app
        .clone()
        .oneshot(request(
            ADMIN_TOKEN,
            workspace.id,
            workspace.generation + 1,
            "stale",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    for ratio in [0, 1001] {
        let mut invalid = request(
            ADMIN_TOKEN,
            workspace.id,
            workspace.generation,
            &format!("invalid-{ratio}"),
        );
        *invalid.body_mut() = Body::from(
            json!({"cpu_overcommit_ratio": ratio, "expected_generation": workspace.generation})
                .to_string(),
        );
        assert_eq!(
            app.clone().oneshot(invalid).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        database.get_workspace(workspace.id).await.unwrap(),
        workspace
    );
    let starting = database
        .request_workspace_action(workspace.id, WorkspaceAction::Start, workspace.owner_id, 10)
        .await
        .unwrap();
    let response = app
        .oneshot(request(
            ADMIN_TOKEN,
            workspace.id,
            starting.generation,
            "running",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        database.get_workspace(workspace.id).await.unwrap(),
        starting
    );
}

#[tokio::test]
async fn reused_idempotency_key_with_different_ratio_is_rejected() {
    let (app, database, workspace) = app().await;
    assert_eq!(
        app.clone()
            .oneshot(request(
                ADMIN_TOKEN,
                workspace.id,
                workspace.generation,
                "ratio-conflict"
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let mut different = request(
        ADMIN_TOKEN,
        workspace.id,
        workspace.generation,
        "ratio-conflict",
    );
    *different.body_mut() = Body::from(
        json!({"cpu_overcommit_ratio": 30, "expected_generation": workspace.generation})
            .to_string(),
    );
    assert_eq!(
        app.oneshot(different).await.unwrap().status(),
        StatusCode::CONFLICT
    );
    let persisted = database.get_workspace(workspace.id).await.unwrap();
    assert_eq!(persisted.generation, workspace.generation + 1);
    assert_eq!(persisted.template.cpu_overcommit_ratio, Some(20));
}

fn builder() -> ResourceBuilder {
    ResourceBuilder {
        installation_id: "cpu-overcommit-api".parse().unwrap(),
        ttyd_image: "tsl0922/ttyd:1.7.7".to_owned(),
        buildkit_image: "moby/buildkit:test".to_owned(),
        ttyd_mtls: None,
        higress_namespace: "higress-system".to_owned(),
        higress_pod_labels: std::collections::BTreeMap::from([(
            "app.kubernetes.io/name".to_owned(),
            "higress-gateway".to_owned(),
        )]),
        higress_source_cidrs: vec!["192.0.2.6/31".to_owned()],
        internet_egress: Some(
            InternetEgressConfig::new(
                "kube-system".to_owned(),
                BTreeMap::from([("k8s-app".to_owned(), "kube-dns".to_owned())]),
                Vec::new(),
            )
            .unwrap(),
        ),
        jump_host_namespace: "workspace-access".to_owned(),
        jump_host_pod_labels: std::collections::BTreeMap::from([(
            "app.kubernetes.io/name".to_owned(),
            "mwc-ssh-jump".to_owned(),
        )]),
        storage_class_name: Some("managed-delete".to_owned()),
        scratch_storage_class_name: "local-nvme-scratch".to_owned(),
        web_shell_domain: Some("shell.example.com".to_owned()),
        port_mapping_domain: Some("ports.example.com".to_owned()),
        higress_gateway_name: "higress-gateway".to_owned(),
        higress_https_section_name: "https".to_owned(),
        internal_ssh_node_port_enabled: false,
    }
}

#[tokio::test]
async fn administrator_keys_need_manage_system_and_must_be_unrestricted() {
    let (app, database, workspace) = app().await;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    use memeloop_workspace_control::auth::ApiKeyScope;
    for (name, scopes, restriction) in [
        (
            "without manage-system",
            vec![ApiKeyScope::ReadWorkspace],
            None,
        ),
        (
            "template restricted admin",
            vec![ApiKeyScope::ManageSystem],
            Some(vec![workspace.template_id.unwrap()]),
        ),
    ] {
        let key = database
            .create_api_key(
                workspace.owner_id,
                name,
                scopes,
                Some(now + 3600),
                restriction,
                now,
            )
            .await
            .unwrap();
        let response = app
            .clone()
            .oneshot(request(
                &key.token,
                workspace.id,
                workspace.generation,
                name,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    assert_eq!(
        database.get_workspace(workspace.id).await.unwrap(),
        workspace
    );
}

#[tokio::test]
async fn active_storage_maintenance_blocks_ratio_updates() {
    let (app, database, workspace) = app().await;
    let Database::Sqlite {
        pool,
        installation_id,
    } = &database
    else {
        unreachable!()
    };
    sqlx::query("INSERT INTO workspace_storage_shrinks (id, installation_id, workspace_id, operation_json, active, created_at, updated_at) VALUES (?1,?2,?3,'{}',1,7,7)")
        .bind(uuid::Uuid::now_v7().to_string()).bind(installation_id.as_str()).bind(workspace.id.to_string()).execute(pool).await.unwrap();
    let response = app
        .oneshot(request(
            ADMIN_TOKEN,
            workspace.id,
            workspace.generation,
            "during-maintenance",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        database.get_workspace(workspace.id).await.unwrap(),
        workspace
    );
}

#[tokio::test]
async fn openapi_describes_the_stopped_workspace_ratio_update() {
    let (app, _, _) = app().await;
    let response = app
        .oneshot(
            Request::get("/api/v1/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, bytes) = body(response).await;
    assert_eq!(status, StatusCode::OK);
    let document: Value = serde_json::from_slice(&bytes).unwrap();
    let operation = &document["paths"]["/api/v1/workspaces/{workspace_id}/cpu-overcommit"]["put"];
    assert_eq!(
        operation["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/UpdateWorkspaceCpuOvercommitRequest"
    );
    let property = &document["components"]["schemas"]["UpdateWorkspaceCpuOvercommitRequest"]["properties"]
        ["cpu_overcommit_ratio"];
    assert_eq!(property["minimum"], 1);
    assert_eq!(property["maximum"], 1000);
}
