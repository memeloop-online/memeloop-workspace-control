use std::{convert::Infallible, sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    http::{Request, Response, StatusCode},
};
use memeloop_workspace_control::{
    api::router,
    auth::ApiKeyScope,
    storage::{Database, StorageError},
    templates::WorkspaceTemplateDocument,
    workspaces::{WorkspaceAction, WorkspaceState},
};
use serde_json::{Value, json};
use sqlx::Row;
use tower::{ServiceExt, service_fn};

use super::support::{ADMIN, Fixture, MEMBER, NAMESPACE, decode};

fn claim(fixture: &Fixture, capacity: &str) -> Value {
    let mut claim = fixture.claim();
    claim["metadata"]["name"] = json!(format!("workspace-data-w-{}-0", fixture.workspace.short_id));
    claim["metadata"]["labels"] = json!({
        "workspace.memeloop.dev/owner-installation":"home-binding",
        "workspace.memeloop.dev/workspace-id":fixture.workspace.id,
    });
    claim["spec"]["resources"]["requests"]["storage"] = json!(capacity);
    claim["status"]["capacity"]["storage"] = json!(capacity);
    claim
}

fn workload(fixture: &Fixture) -> Value {
    let (_, mut sts) = fixture.adopted_runtime();
    sts["spec"]["replicas"] = json!(0);
    sts["spec"]["template"]["spec"]["volumes"] = json!([]);
    sts["spec"]["volumeClaimTemplates"] = json!([{
        "metadata":{"name":"workspace-data"},
        "spec":{"accessModes":["ReadWriteOnce"],"resources":{"requests":{"storage":format!("{}Gi",fixture.workspace.template.resources.disk_gib)}}},
    }]);
    sts
}

fn app(fixture: &Fixture, claim: Value, sts: Value, pods: Vec<Value>) -> Router {
    let name = format!("w-{}", fixture.workspace.short_id);
    let service = service_fn(move |request: Request<kube::client::Body>| {
        let (claim, sts, pods, name) = (claim.clone(), sts.clone(), pods.clone(), name.clone());
        async move {
            assert_eq!(
                request.method(),
                "GET",
                "capacity correction must never mutate Kubernetes"
            );
            let path = request.uri().path();
            let (status, body) = if path
                .ends_with(&format!("/persistentvolumeclaims/workspace-data-{name}-0"))
                && !claim.is_null()
            {
                (StatusCode::OK, claim)
            } else if path.ends_with("/pods") {
                (
                    StatusCode::OK,
                    json!({"apiVersion":"v1","kind":"PodList","metadata":{},"items":pods}),
                )
            } else if path.ends_with(&format!("/pods/{name}-0")) && !pods.is_empty() {
                (StatusCode::OK, pods[0].clone())
            } else if path.ends_with("/statefulsets") {
                let items = if sts.is_null() {
                    vec![]
                } else {
                    vec![sts.clone()]
                };
                (
                    StatusCode::OK,
                    json!({"apiVersion":"apps/v1","kind":"StatefulSetList","metadata":{},"items":items}),
                )
            } else if path.ends_with(&format!("/statefulsets/{name}")) && !sts.is_null() {
                (StatusCode::OK, sts)
            } else {
                (
                    StatusCode::NOT_FOUND,
                    json!({"apiVersion":"v1","kind":"Status","status":"Failure","reason":"NotFound","message":"fixture missing","code":404}),
                )
            };
            Ok::<_, Infallible>(
                Response::builder()
                    .status(status)
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
        }
    });
    let mut state = fixture.state();
    state.set_kubernetes_client(kube::Client::new(service, NAMESPACE));
    router(Arc::new(state))
}

fn input(fixture: &Fixture, capacity: u64) -> Value {
    json!({"capacity_gib":capacity,"claim_uid":fixture.binding().claim_uid,"expected_generation":fixture.workspace.generation})
}

async fn put(
    app: Router,
    fixture: &Fixture,
    token: &str,
    body: Value,
    key: &str,
) -> (StatusCode, Value) {
    decode(
        app.oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!(
                    "/api/v1/workspaces/{}/home-capacity",
                    fixture.workspace.id
                ))
                .header("Authorization", format!("Bearer {token}"))
                .header("Content-Type", "application/json")
                .header("Idempotency-Key", key)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap(),
    )
    .await
}

#[tokio::test]
async fn corrects_three_migration_sizes_without_binding_or_shared_template_changes() {
    for (previous, target) in [(100, 40), (30, 40), (2, 4)] {
        let mut fixture = Fixture::new().await;
        fixture.workspace = fixture
            .database
            .correct_stopped_workspace_home_capacity(
                fixture.workspace.id,
                previous,
                fixture.workspace.generation,
                fixture.workspace.owner_id,
                fixture.now,
                &fixture.binding().claim_uid,
            )
            .await
            .unwrap();
        let template = fixture
            .database
            .get_workspace_template(fixture.workspace.template_id.unwrap())
            .await
            .unwrap();
        let before = fixture.database.job_counts().await.unwrap().pending;
        let app = app(
            &fixture,
            claim(&fixture, &format!("{target}Gi")),
            workload(&fixture),
            vec![],
        );
        let result = put(
            app.clone(),
            &fixture,
            ADMIN,
            input(&fixture, target),
            "correct",
        )
        .await;
        assert_eq!(result.0, StatusCode::OK, "{}", result.1);
        assert_eq!(result.1["workspace"]["resources"]["disk_gib"], target);
        assert!(result.1["workspace"]["home_volume_binding"].is_null());
        assert_eq!(
            put(
                app.clone(),
                &fixture,
                ADMIN,
                input(&fixture, target),
                "correct"
            )
            .await,
            result
        );
        assert_eq!(
            put(app, &fixture, ADMIN, input(&fixture, target), "stale")
                .await
                .0,
            StatusCode::CONFLICT
        );
        let stored = fixture
            .database
            .get_workspace(fixture.workspace.id)
            .await
            .unwrap();
        assert_eq!(stored.state, WorkspaceState::Stopped);
        assert_eq!(stored.generation, fixture.workspace.generation + 1);
        assert_eq!(
            stored.template.storage_policy,
            fixture.workspace.template.storage_policy
        );
        assert!(stored.home_volume_binding.is_none());
        assert_eq!(
            serde_json::to_value(
                fixture
                    .database
                    .get_workspace_template(fixture.workspace.template_id.unwrap())
                    .await
                    .unwrap()
            )
            .unwrap(),
            serde_json::to_value(template).unwrap()
        );
        assert_eq!(
            fixture.database.job_counts().await.unwrap().pending,
            before + 1
        );
        let Database::Sqlite { pool, .. } = &fixture.database else {
            unreachable!()
        };
        let row = sqlx::query(
            "SELECT disk_gib, template_snapshot_yaml, home_pvc_name FROM workspaces WHERE id = ?1",
        )
        .bind(stored.id.to_string())
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(row.get::<i64, _>("disk_gib"), target as i64);
        assert!(row.get::<Option<String>, _>("home_pvc_name").is_none());
        assert_eq!(
            WorkspaceTemplateDocument::parse(&row.get::<String, _>("template_snapshot_yaml"))
                .unwrap()
                .spec
                .resources
                .disk_gib,
            target
        );
        let audit: String = sqlx::query_scalar("SELECT metadata_json FROM audit_log WHERE workspace_id = ?1 AND action = 'workspace.home_capacity_corrected' ORDER BY rowid DESC LIMIT 1")
            .bind(stored.id.to_string()).fetch_one(pool).await.unwrap();
        let audit: Value = serde_json::from_str(&audit).unwrap();
        assert_eq!(audit["previous_disk_gib"], previous);
        assert_eq!(audit["disk_gib"], target);
        assert_eq!(audit["claim_uid"], fixture.binding().claim_uid);
    }
}

#[tokio::test]
async fn requires_unrestricted_system_admin_and_rejects_extra_claim_selection_fields() {
    let fixture = Fixture::new().await;
    let app = app(
        &fixture,
        claim(&fixture, "40Gi"),
        workload(&fixture),
        vec![],
    );
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
    let readonly = fixture
        .database
        .create_api_key(
            fixture.workspace.owner_id,
            "readonly",
            vec![ApiKeyScope::ReadWorkspace],
            Some(fixture.now + 3600),
            None,
            fixture.now,
        )
        .await
        .unwrap();
    for token in [MEMBER, restricted.token.as_str(), readonly.token.as_str()] {
        assert_eq!(
            put(app.clone(), &fixture, token, input(&fixture, 40), "denied")
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    let mut extra = input(&fixture, 40);
    extra["claim_name"] = json!("another-users-home");
    assert_eq!(
        put(app, &fixture, ADMIN, extra, "extra").await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        fixture
            .database
            .get_workspace(fixture.workspace.id)
            .await
            .unwrap(),
        fixture.workspace
    );
}

#[tokio::test]
async fn rejects_missing_replaced_unbound_wrong_capacity_and_unowned_claims() {
    let fixture = Fixture::new().await;
    let original = claim(&fixture, "40Gi");
    let mut cases = vec![(Value::Null, StatusCode::UNPROCESSABLE_ENTITY)];
    for (pointer, value) in [
        ("/metadata/uid", json!("replacement-uid")),
        ("/metadata/namespace", json!("other-namespace")),
        ("/metadata/deletionTimestamp", json!("2026-10-04T00:00:00Z")),
        ("/status/phase", json!("Pending")),
        ("/status/capacity/storage", json!("39Gi")),
        ("/spec/resources/requests/storage", json!("41Gi")),
    ] {
        let mut invalid = original.clone();
        if pointer == "/metadata/deletionTimestamp" {
            invalid["metadata"]["deletionTimestamp"] = value;
        } else {
            *invalid.pointer_mut(pointer).unwrap() = value;
        }
        cases.push((invalid, StatusCode::UNPROCESSABLE_ENTITY));
    }
    let mut unowned = original.clone();
    unowned["metadata"]["labels"] = json!({});
    cases.push((unowned, StatusCode::CONFLICT));
    let mut foreign = original.clone();
    foreign["metadata"]["labels"]["workspace.memeloop.dev/owner-installation"] = json!("other");
    cases.push((foreign, StatusCode::CONFLICT));
    let mut controlled = original;
    controlled["metadata"]["ownerReferences"] = json!([{"apiVersion":"apps/v1","kind":"StatefulSet","name":"old","uid":"old-uid","controller":true}]);
    cases.push((controlled, StatusCode::CONFLICT));
    for (claim, expected) in cases {
        let result = put(
            app(&fixture, claim, workload(&fixture), vec![]),
            &fixture,
            ADMIN,
            input(&fixture, 40),
            "retry",
        )
        .await;
        assert_eq!(result.0, expected, "{}", result.1);
        assert_eq!(
            fixture
                .database
                .get_workspace(fixture.workspace.id)
                .await
                .unwrap(),
            fixture.workspace
        );
    }
    for quantity in ["40Gi", "40.0Gi", "40960Mi", "4.294967296e10"] {
        let fixture = Fixture::new().await;
        let result = put(
            app(
                &fixture,
                claim(&fixture, quantity),
                workload(&fixture),
                vec![],
            ),
            &fixture,
            ADMIN,
            input(&fixture, 40),
            "equivalent",
        )
        .await;
        assert_eq!(result.0, StatusCode::OK, "{}", result.1);
    }
}

#[tokio::test]
async fn refuses_live_pods_unsafe_retention_custom_homes_and_missing_workloads() {
    let fixture = Fixture::new().await;
    let sts = workload(&fixture);
    let mut cases = vec![Value::Null];
    let mut active = sts.clone();
    active["spec"]["replicas"] = json!(1);
    cases.push(active);
    let mut destructive = sts.clone();
    destructive["spec"]["persistentVolumeClaimRetentionPolicy"] =
        json!({"whenDeleted":"Delete","whenScaled":"Retain"});
    cases.push(destructive);
    let mut custom = sts.clone();
    custom["spec"]["volumeClaimTemplates"] = json!([]);
    cases.push(custom);
    let mut explicit = sts.clone();
    explicit["spec"]["template"]["spec"]["volumes"] =
        json!([{"name":"workspace-data","persistentVolumeClaim":{"claimName":"custom-home"}}]);
    cases.push(explicit);
    let mut unowned = sts;
    unowned["metadata"]["labels"] = json!({});
    cases.push(unowned);
    for sts in cases {
        assert_eq!(
            put(
                app(&fixture, claim(&fixture, "40Gi"), sts, vec![]),
                &fixture,
                ADMIN,
                input(&fixture, 40),
                "unsafe"
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
    }
    let (mut pod, _) = fixture.adopted_runtime();
    pod["metadata"]["labels"] = json!({});
    assert_eq!(
        put(
            app(
                &fixture,
                claim(&fixture, "40Gi"),
                workload(&fixture),
                vec![pod]
            ),
            &fixture,
            ADMIN,
            input(&fixture, 40),
            "unlabelled-pod"
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        fixture
            .database
            .get_workspace(fixture.workspace.id)
            .await
            .unwrap(),
        fixture.workspace
    );
}

#[tokio::test]
async fn enforces_lease_generation_and_refuses_ready_or_external_binding() {
    let mut fixture = Fixture::new().await;
    let app = app(
        &fixture,
        claim(&fixture, "40Gi"),
        workload(&fixture),
        vec![],
    );
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
    assert_eq!(
        put(app.clone(), &fixture, ADMIN, input(&fixture, 40), "lease")
            .await
            .0,
        StatusCode::CONFLICT
    );
    fixture
        .database
        .release_workspace_lease(fixture.workspace.id, "worker")
        .await
        .unwrap();
    let (first, second) = tokio::join!(
        put(app.clone(), &fixture, ADMIN, input(&fixture, 40), "race-a"),
        put(app.clone(), &fixture, ADMIN, input(&fixture, 40), "race-b")
    );
    assert!([first.0, second.0].contains(&StatusCode::OK));
    assert!([first.0, second.0].contains(&StatusCode::CONFLICT));
    fixture.workspace = fixture
        .database
        .get_workspace(fixture.workspace.id)
        .await
        .unwrap();
    fixture.ready().await;
    assert_eq!(
        put(app.clone(), &fixture, ADMIN, input(&fixture, 40), "ready")
            .await
            .0,
        StatusCode::CONFLICT
    );
    fixture.terminal_failure().await;
    assert_eq!(
        put(app, &fixture, ADMIN, input(&fixture, 40), "failed")
            .await
            .0,
        StatusCode::CONFLICT
    );
    let mut external = Fixture::new().await;
    external.workspace = external
        .database
        .bind_workspace_home_volume(
            external.workspace.id,
            &external.binding(),
            external.workspace.generation,
            external.workspace.owner_id,
            external.now,
            false,
        )
        .await
        .unwrap();
    let result = external
        .database
        .correct_stopped_workspace_home_capacity(
            external.workspace.id,
            40,
            external.workspace.generation,
            external.workspace.owner_id,
            external.now,
            &external.binding().claim_uid,
        )
        .await;
    assert!(matches!(
        result,
        Err(StorageError::WorkspaceHomePvcUpdateConflict)
    ));
}

#[tokio::test]
async fn concurrent_stop_or_delete_generation_cannot_be_overwritten() {
    for action in [WorkspaceAction::Start, WorkspaceAction::Delete] {
        let fixture = Fixture::new().await;
        fixture
            .database
            .request_workspace_action(
                fixture.workspace.id,
                action,
                fixture.workspace.owner_id,
                fixture.now,
            )
            .await
            .unwrap();
        let result = put(
            app(
                &fixture,
                claim(&fixture, "40Gi"),
                workload(&fixture),
                vec![],
            ),
            &fixture,
            ADMIN,
            input(&fixture, 40),
            "action-race",
        )
        .await;
        assert_eq!(result.0, StatusCode::CONFLICT);
        assert_eq!(
            fixture
                .database
                .get_workspace(fixture.workspace.id)
                .await
                .unwrap()
                .template
                .resources
                .disk_gib,
            100
        );
    }
}

#[tokio::test]
async fn postgres_capacity_cas_is_atomic_and_persists_without_binding() {
    let Ok(url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        return;
    };
    let schema = format!("mwc_home_capacity_{}", uuid::Uuid::now_v7().simple());
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
    let correction = || {
        fixture.database.correct_stopped_workspace_home_capacity(
            fixture.workspace.id,
            40,
            fixture.workspace.generation,
            fixture.workspace.owner_id,
            fixture.now,
            "pvc-original-uid",
        )
    };
    let (first, second) = tokio::join!(correction(), correction());
    assert!(matches!(
        (first, second),
        (Ok(_), Err(StorageError::WorkspaceHomePvcUpdateConflict))
            | (Err(StorageError::WorkspaceHomePvcUpdateConflict), Ok(_))
    ));
    let readback = Database::connect(scoped.as_str(), "home-binding".parse().unwrap())
        .await
        .unwrap();
    let stored = readback.get_workspace(fixture.workspace.id).await.unwrap();
    assert_eq!(stored.template.resources.disk_gib, 40);
    assert!(stored.home_volume_binding.is_none());
    let Database::Postgres { pool, .. } = &readback else {
        unreachable!()
    };
    let accounting: i64 = sqlx::query_scalar("SELECT disk_gib FROM workspaces WHERE id=$1")
        .bind(stored.id.to_string())
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(accounting, 40);
    drop(readback);
    drop(fixture);
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
}
