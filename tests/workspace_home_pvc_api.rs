#[path = "support/home_capacity.rs"]
mod managed_capacity;
#[path = "support/home_pvc_pending.rs"]
mod pending;
#[path = "support/home_pvc.rs"]
mod support;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use memeloop_workspace_control::{
    api::router,
    auth::ApiKeyScope,
    storage::{CreateWorkspace, Database, StorageError},
    templates::WorkspaceTemplateDocument,
    workspaces::{WorkspaceAction, WorkspaceObservation},
};
use serde_json::{Value, json};
use sqlx::Row;
use std::{sync::Arc, time::Duration};
use support::{ADMIN, Fixture, MEMBER, decode, put};
use tower::ServiceExt;

#[tokio::test]
async fn binds_stopped_home_atomically_and_replays_without_another_job() {
    let fixture = Fixture::new().await;
    let app = fixture.app(fixture.claim(), vec![], vec![]);
    let (status, response) = put(
        app.clone(),
        &fixture.workspace,
        ADMIN,
        fixture.input(),
        "bind-1",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(
        response["workspace"]["home_volume_binding"],
        serde_json::to_value(fixture.binding()).unwrap()
    );
    assert_eq!(response["workspace"]["resources"]["disk_gib"], 20);
    assert_eq!(
        response["workspace"]["generation"],
        fixture.workspace.generation + 1
    );
    let replay = put(
        app.clone(),
        &fixture.workspace,
        ADMIN,
        fixture.input(),
        "bind-1",
    )
    .await;
    assert_eq!(replay, (status, response));
    let (status, _) = put(app, &fixture.workspace, ADMIN, fixture.input(), "stale").await;
    assert_eq!(status, StatusCode::CONFLICT);
    let stored = fixture
        .database
        .get_workspace(fixture.workspace.id)
        .await
        .unwrap();
    assert_eq!(stored.home_volume_binding, Some(fixture.binding()));
    let Database::Sqlite { pool, .. } = &fixture.database else {
        unreachable!()
    };
    let row = sqlx::query("SELECT disk_gib, template_snapshot_yaml FROM workspaces WHERE id = ?1")
        .bind(stored.id.to_string())
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(row.get::<i64, _>("disk_gib"), 20);
    let snapshot =
        WorkspaceTemplateDocument::parse(&row.get::<String, _>("template_snapshot_yaml")).unwrap();
    assert_eq!(snapshot.spec.resources.disk_gib, 20);
    let jobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE workspace_id = ?1 AND payload_json LIKE '%home_volume_bound%'").bind(stored.id.to_string()).fetch_one(pool).await.unwrap();
    let audit: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE workspace_id = ?1 AND action = 'workspace.home_volume_bound'").bind(stored.id.to_string()).fetch_one(pool).await.unwrap();
    assert_eq!((jobs, audit), (1, 1));
    let export = fixture.database.export_snapshot(fixture.now).await.unwrap();
    assert_eq!(export.tables["workspaces"][0]["home_pvc_capacity_gib"], 20);
}

#[tokio::test]
async fn only_unrestricted_system_administrators_can_bind() {
    let fixture = Fixture::new().await;
    let app = fixture.app(fixture.claim(), vec![], vec![]);
    let restricted = fixture
        .database
        .create_api_key(
            fixture.workspace.owner_id,
            "restricted",
            vec![ApiKeyScope::ManageSystem],
            Some(fixture.now + 3600),
            Some(vec![fixture.workspace.template_id.unwrap()]),
            fixture.now,
        )
        .await
        .unwrap();
    let unprivileged = fixture
        .database
        .create_api_key(
            fixture.workspace.owner_id,
            "read only",
            vec![ApiKeyScope::ReadWorkspace],
            Some(fixture.now + 3600),
            None,
            fixture.now,
        )
        .await
        .unwrap();
    for token in [
        MEMBER,
        restricted.token.as_str(),
        unprivileged.token.as_str(),
    ] {
        let (status, _) = put(
            app.clone(),
            &fixture.workspace,
            token,
            fixture.input(),
            "denied",
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    assert!(
        fixture
            .database
            .get_workspace(fixture.workspace.id)
            .await
            .unwrap()
            .home_volume_binding
            .is_none()
    );
}

#[tokio::test]
async fn validates_namespace_uid_phase_capacity_and_releases_failed_idempotency_reservation() {
    let fixture = Fixture::new().await;
    for (field, value) in [
        ("namespace", json!("another-namespace")),
        ("claim_name", json!("../secret")),
        ("capacity_gib", json!(0)),
    ] {
        let mut input = fixture.input();
        input[field] = value;
        let (status, _) = put(
            fixture.app(fixture.claim(), vec![], vec![]),
            &fixture.workspace,
            ADMIN,
            input,
            "invalid",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let mut invalid_claims = vec![Value::Null];
    for (pointer, value) in [
        ("/metadata/uid", json!("replacement-uid")),
        ("/metadata/deletionTimestamp", json!("2026-10-04T00:00:00Z")),
        ("/status/phase", json!("Pending")),
        ("/status/capacity/storage", json!("100Gi")),
        ("/spec/resources/requests/storage", json!("100Gi")),
        ("/spec/volumeMode", json!("Block")),
    ] {
        let mut claim = fixture.claim();
        let (parent, field) = pointer.rsplit_once('/').unwrap();
        claim.pointer_mut(parent).unwrap()[field] = value;
        invalid_claims.push(claim);
    }
    for claim in invalid_claims {
        let (status, response) = put(
            fixture.app(claim, vec![], vec![]),
            &fixture.workspace,
            ADMIN,
            fixture.input(),
            "retryable",
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{response}");
    }
    let mut claim = fixture.claim();
    claim["status"]["capacity"]["storage"] = json!("20480Mi");
    let (status, response) = put(
        fixture.app(claim, vec![], vec![]),
        &fixture.workspace,
        ADMIN,
        fixture.input(),
        "retryable",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
}

#[tokio::test]
async fn rejects_claim_ownership_mounts_and_missing_kubernetes() {
    let fixture = Fixture::new().await;
    let mut claim = fixture.claim();
    claim["metadata"]["labels"] =
        json!({"workspace.memeloop.dev/workspace-id":uuid::Uuid::now_v7()});
    let (status, _) = put(
        fixture.app(claim, vec![], vec![]),
        &fixture.workspace,
        ADMIN,
        fixture.input(),
        "owner",
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (pod, sts) = fixture.adopted_runtime();
    let (status, _) = put(
        fixture.app(fixture.claim(), vec![pod], vec![sts]),
        &fixture.workspace,
        ADMIN,
        fixture.input(),
        "mounted",
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = put(
        router(Arc::new(fixture.state())),
        &fixture.workspace,
        ADMIN,
        fixture.input(),
        "offline",
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn adopts_identical_ready_home_without_enqueueing_reconcile_or_mutating_kubernetes() {
    adopts_current_home(false).await;
}

#[tokio::test]
async fn failed_workspace_with_ready_current_home_recovers_atomically() {
    adopts_current_home(true).await;
}

async fn adopts_current_home(terminal_failure: bool) {
    let mut fixture = Fixture::new().await;
    fixture.ready().await;
    if terminal_failure {
        fixture.terminal_failure().await;
    }
    let (pod, sts) = fixture.adopted_runtime();
    let app = fixture.app(fixture.claim(), vec![pod], vec![sts]);
    let (status, response) = put(
        app.clone(),
        &fixture.workspace,
        ADMIN,
        fixture.input(),
        "adopt",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["workspace"]["state"], "ready");
    assert_eq!(response["workspace"]["resources"]["disk_gib"], 20);
    let stored = fixture
        .database
        .get_workspace(fixture.workspace.id)
        .await
        .unwrap();
    assert_eq!(stored.home_volume_binding, Some(fixture.binding()));
    assert_eq!(fixture.database.job_counts().await.unwrap().pending, 0);
    let (status, _) = put(
        app,
        &fixture.workspace,
        ADMIN,
        fixture.input(),
        "adopt-stale",
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn ready_adoption_rejects_different_live_home_templates_and_container_mounts() {
    rejects_unsafe_adoption(false).await;
}

#[tokio::test]
async fn failed_adoption_rejects_different_live_home_templates_and_container_mounts() {
    rejects_unsafe_adoption(true).await;
}

async fn rejects_unsafe_adoption(terminal_failure: bool) {
    let mut fixture = Fixture::new().await;
    fixture.ready().await;
    if terminal_failure {
        fixture.terminal_failure().await;
    }
    let (pod, sts) = fixture.adopted_runtime();
    let mut cases = Vec::new();
    let mut different_pod = pod.clone();
    different_pod["spec"]["volumes"][0]["persistentVolumeClaim"]["claimName"] = json!("old-home");
    cases.push((different_pod, sts.clone()));
    let mut different_sts = sts.clone();
    different_sts["spec"]["template"]["spec"]["volumes"][0]["persistentVolumeClaim"]["claimName"] =
        json!("old-home");
    cases.push((pod.clone(), different_sts));
    let mut templated = sts.clone();
    templated["spec"]["volumeClaimTemplates"] = json!([{"metadata":{"name":"workspace-data"}}]);
    cases.push((pod.clone(), templated));
    let mut sidecar = pod.clone();
    sidecar["spec"]["containers"].as_array_mut().unwrap().push(json!({"name":"sidecar","volumeMounts":[{"name":"old-home","mountPath":fixture.workspace.template.workspace_home}]}));
    cases.push((sidecar, sts.clone()));
    let mut unready = pod.clone();
    unready["status"]["conditions"][0]["status"] = json!("False");
    cases.push((unready, sts));
    for (pod, sts) in cases {
        let (status, response) = put(
            fixture.app(fixture.claim(), vec![pod], vec![sts]),
            &fixture.workspace,
            ADMIN,
            fixture.input(),
            "not-adoptable",
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{response}");
    }
    assert!(
        fixture
            .database
            .get_workspace(fixture.workspace.id)
            .await
            .unwrap()
            .home_volume_binding
            .is_none()
    );
}

#[tokio::test]
async fn lease_and_generation_conflicts_do_not_commit_partial_binding() {
    binding_race(false).await;
}

#[tokio::test]
async fn failed_adoption_lease_and_generation_conflicts_do_not_commit_partial_binding() {
    binding_race(true).await;
}

async fn binding_race(terminal_failure: bool) {
    let mut fixture = Fixture::new().await;
    let app = if terminal_failure {
        fixture.ready().await;
        fixture.terminal_failure().await;
        let (pod, sts) = fixture.adopted_runtime();
        fixture.app(fixture.claim(), vec![pod], vec![sts])
    } else {
        fixture.app(fixture.claim(), vec![], vec![])
    };
    assert!(
        fixture
            .database
            .try_acquire_workspace_lease(
                fixture.workspace.id,
                "worker",
                fixture.now,
                Duration::from_secs(60)
            )
            .await
            .unwrap()
    );
    let (status, _) = put(
        app.clone(),
        &fixture.workspace,
        ADMIN,
        fixture.input(),
        "lease",
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    fixture
        .database
        .release_workspace_lease(fixture.workspace.id, "worker")
        .await
        .unwrap();
    let (first, second) = tokio::join!(
        put(
            app.clone(),
            &fixture.workspace,
            ADMIN,
            fixture.input(),
            "race-1"
        ),
        put(app, &fixture.workspace, ADMIN, fixture.input(), "race-2")
    );
    assert!([first.0, second.0].contains(&StatusCode::OK));
    assert!([first.0, second.0].contains(&StatusCode::CONFLICT));
    let stored = fixture
        .database
        .get_workspace(fixture.workspace.id)
        .await
        .unwrap();
    assert_eq!(stored.generation, fixture.workspace.generation + 1);
}

#[tokio::test]
async fn database_binding_conflicts_and_runtime_capacity_follow_the_bound_claim() {
    let fixture = Fixture::new().await;
    let app = fixture.app(fixture.claim(), vec![], vec![]);
    assert_eq!(
        put(
            app.clone(),
            &fixture.workspace,
            ADMIN,
            fixture.input(),
            "runtime-bind"
        )
        .await
        .0,
        StatusCode::OK
    );
    let request = Request::builder()
        .uri(format!(
            "/api/v1/workspaces/{}/runtime",
            fixture.workspace.id
        ))
        .header("Authorization", format!("Bearer {ADMIN}"))
        .body(Body::empty())
        .unwrap();
    let (status, response) = decode(app.oneshot(request).await.unwrap()).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["allocated"]["disk_gib"], 20);
    assert_eq!(
        response["persistent_storage"]["configured_bytes"],
        20_u64 * (1 << 30)
    );
    assert!(matches!(
        fixture
            .database
            .bind_workspace_home_volume(
                fixture.workspace.id,
                &fixture.binding(),
                fixture.workspace.generation,
                fixture.workspace.owner_id,
                fixture.now,
                false
            )
            .await,
        Err(StorageError::WorkspaceHomePvcUpdateConflict)
    ));
}

async fn second_workspace(fixture: &Fixture) -> memeloop_workspace_control::workspaces::Workspace {
    let workspace = fixture
        .database
        .create_workspace(
            CreateWorkspace {
                organization_id: fixture.workspace.organization_id,
                owner_id: fixture.workspace.owner_id,
                name: "Another workspace".to_owned(),
                template_id: fixture.workspace.template_id.unwrap(),
                resources: None,
                organization_injection_refs: None,
                user_injection_refs: None,
            },
            true,
            fixture.workspace.owner_id,
            fixture.now,
        )
        .await
        .unwrap();
    fixture
        .database
        .request_workspace_action(
            workspace.id,
            WorkspaceAction::Stop,
            workspace.owner_id,
            fixture.now,
        )
        .await
        .unwrap();
    fixture
        .database
        .record_workspace_observation(
            workspace.id,
            WorkspaceObservation::Stopped,
            workspace.owner_id,
            fixture.now,
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn storage_rejects_double_booking_and_another_workspaces_generated_claim() {
    let fixture = Fixture::new().await;
    let other = second_workspace(&fixture).await;
    let mut binding = fixture.binding();
    binding.claim_name = format!("workspace-data-w-{}-0", other.short_id);
    assert!(matches!(
        fixture
            .database
            .bind_workspace_home_volume(
                fixture.workspace.id,
                &binding,
                fixture.workspace.generation,
                fixture.workspace.owner_id,
                fixture.now,
                false
            )
            .await,
        Err(StorageError::WorkspaceHomePvcInUse)
    ));
    let binding = fixture.binding();
    fixture
        .database
        .bind_workspace_home_volume(
            fixture.workspace.id,
            &binding,
            fixture.workspace.generation,
            fixture.workspace.owner_id,
            fixture.now,
            false,
        )
        .await
        .unwrap();
    assert!(matches!(
        fixture
            .database
            .bind_workspace_home_volume(
                other.id,
                &binding,
                other.generation,
                other.owner_id,
                fixture.now,
                false
            )
            .await,
        Err(StorageError::WorkspaceHomePvcInUse)
    ));
    assert!(
        fixture
            .database
            .get_workspace(other.id)
            .await
            .unwrap()
            .home_volume_binding
            .is_none()
    );
}

#[tokio::test]
async fn ready_adoption_rejects_running_reconcile_and_foreign_pod() {
    rejects_busy_adoption(false).await;
}

#[tokio::test]
async fn failed_adoption_rejects_running_reconcile_and_foreign_pod() {
    rejects_busy_adoption(true).await;
}

async fn rejects_busy_adoption(terminal_failure: bool) {
    let mut fixture = Fixture::new().await;
    fixture.ready().await;
    if terminal_failure {
        fixture.terminal_failure().await;
    }
    let (pod, sts) = fixture.adopted_runtime();
    let mut foreign = pod.clone();
    foreign["metadata"]["name"] = json!("foreign-pod");
    foreign["metadata"]["uid"] = json!("foreign-uid");
    foreign["metadata"]["labels"] = json!({});
    let (status, _) = put(
        fixture.app(
            fixture.claim(),
            vec![pod.clone(), foreign],
            vec![sts.clone()],
        ),
        &fixture.workspace,
        ADMIN,
        fixture.input(),
        "foreign",
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let Database::Sqlite { pool, .. } = &fixture.database else {
        unreachable!()
    };
    sqlx::query("UPDATE jobs SET status = 'running' WHERE workspace_id = ?1")
        .bind(fixture.workspace.id.to_string())
        .execute(pool)
        .await
        .unwrap();
    let (status, _) = put(
        fixture.app(fixture.claim(), vec![pod], vec![sts]),
        &fixture.workspace,
        ADMIN,
        fixture.input(),
        "running",
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn postgres_binding_competition_is_atomic_and_survives_database_reconnect() {
    let Ok(url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        return;
    };
    let schema = format!("mwc_home_pvc_{}", uuid::Uuid::now_v7().simple());
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let mut scoped = url::Url::parse(&url).unwrap();
    scoped
        .query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema}"));
    let database = Database::connect(scoped.as_str(), "home-binding".parse().unwrap())
        .await
        .unwrap();
    let fixture = Fixture::with_database(database).await;
    downgrade_binding_schema(&fixture.database).await;
    fixture.database.migrate().await.unwrap();
    assert_eq!(
        fixture
            .database
            .get_workspace(fixture.workspace.id)
            .await
            .unwrap(),
        fixture.workspace
    );
    let other = second_workspace(&fixture).await;
    let binding = fixture.binding();
    let (first, second) = tokio::join!(
        fixture.database.bind_workspace_home_volume(
            fixture.workspace.id,
            &binding,
            fixture.workspace.generation,
            fixture.workspace.owner_id,
            fixture.now,
            false
        ),
        fixture.database.bind_workspace_home_volume(
            other.id,
            &binding,
            other.generation,
            other.owner_id,
            fixture.now,
            false
        )
    );
    let winner = match (first, second) {
        (Ok(workspace), Err(StorageError::WorkspaceHomePvcInUse))
        | (Err(StorageError::WorkspaceHomePvcInUse), Ok(workspace)) => workspace,
        result => panic!("unexpected concurrent binding results: {result:?}"),
    };
    let reconnected = Database::connect(scoped.as_str(), "home-binding".parse().unwrap())
        .await
        .unwrap();
    let readback = reconnected.get_workspace(winner.id).await.unwrap();
    assert_eq!(readback.home_volume_binding, Some(binding));
    assert_eq!(readback.template.resources.disk_gib, 20);
    drop(reconnected);
    drop(fixture);
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
}

async fn downgrade_binding_schema(database: &Database) {
    let statements = [
        "DROP INDEX workspace_home_pvc_name_idx",
        "DROP INDEX workspace_home_pvc_uid_idx",
        "ALTER TABLE workspaces DROP COLUMN home_pvc_namespace",
        "ALTER TABLE workspaces DROP COLUMN home_pvc_name",
        "ALTER TABLE workspaces DROP COLUMN home_pvc_uid",
        "ALTER TABLE workspaces DROP COLUMN home_pvc_capacity_gib",
        "DELETE FROM schema_migrations WHERE version = 26",
        "INSERT INTO schema_migrations (version, applied_at) VALUES (25, 1)",
    ];
    for statement in statements {
        match database {
            Database::Sqlite { pool, .. } => {
                sqlx::query(statement).execute(pool).await.unwrap();
            }
            Database::Postgres { pool, .. } => {
                sqlx::query(statement).execute(pool).await.unwrap();
            }
        }
    }
}

#[tokio::test]
async fn sqlite_v25_upgrade_preserves_workspace_and_does_not_bind_or_change_a_template() {
    let fixture = Fixture::new().await;
    downgrade_binding_schema(&fixture.database).await;
    fixture.database.migrate().await.unwrap();
    fixture.database.migrate().await.unwrap();
    assert_eq!(fixture.database.schema_version().await.unwrap(), 26);
    let stored = fixture
        .database
        .get_workspace(fixture.workspace.id)
        .await
        .unwrap();
    assert_eq!(stored, fixture.workspace);
    let mut document = serde_json::to_value(WorkspaceTemplateDocument::new(
        "unsafe binding",
        stored.template,
    ))
    .unwrap();
    document["spec"]["home_volume_binding"] = serde_json::to_value(fixture.binding()).unwrap();
    assert!(
        WorkspaceTemplateDocument::parse(&serde_yaml_ng::to_string(&document).unwrap()).is_err()
    );
}

#[tokio::test]
async fn accepts_equivalent_exact_kubernetes_quantity_representations() {
    for quantity in ["20Gi", "20.0Gi", "20480Mi", "21474836480", "2.147483648e10"] {
        let fixture = Fixture::new().await;
        let mut claim = fixture.claim();
        claim["spec"]["resources"]["requests"]["storage"] = json!(quantity);
        claim["status"]["capacity"]["storage"] = json!(quantity);
        let (status, response) = put(
            fixture.app(claim, vec![], vec![]),
            &fixture.workspace,
            ADMIN,
            fixture.input(),
            "quantity",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{quantity}: {response}");
    }
}
