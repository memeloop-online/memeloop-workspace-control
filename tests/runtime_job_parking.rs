use std::time::Duration;

use memeloop_workspace_control::{
    config::InstallationId,
    quota::Resources,
    storage::{
        CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate, Database, NewJob,
        StorageError,
    },
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::{AccessMode, Workspace},
};
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

async fn seed(database: &Database) -> Workspace {
    database.migrate().await.unwrap();
    database
        .upsert_image_policy("registry.example/workspace:1", true, 1)
        .await
        .unwrap();
    let admin = database
        .create_user_with_initial_key(
            "Parking administrator",
            "parking-admin-000000000000000000000000000000",
            true,
            memeloop_workspace_control::auth::ApiKeyScope::initial_key_defaults(true),
            31_536_000,
            1,
        )
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Runtime parking".to_owned(),
                owner_user_id: admin.user_id,
            },
            2,
        )
        .await
        .unwrap();
    let template = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(organization.id),
                yaml: WorkspaceTemplateDocument::new(
                    "Runtime parking",
                    WorkspaceTemplateSpec::standard(
                        "registry.example/workspace:1",
                        AccessMode::Internal,
                        Resources {
                            cpu_millis: 500,
                            memory_mib: 512,
                            gpu_count: 0,
                            disk_gib: 10,
                        },
                    ),
                )
                .to_yaml()
                .unwrap(),
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
                name: "parked workspace".to_owned(),
                template_id: template.id,
                resources: None,
                organization_injection_refs: None,
                user_injection_refs: None,
            },
            true,
            admin.user_id,
            4,
        )
        .await
        .unwrap();
    clear_jobs(database).await;
    workspace
}

async fn clear_jobs(database: &Database) {
    match database {
        Database::Sqlite { pool, .. } => {
            sqlx::query("DELETE FROM jobs").execute(pool).await.unwrap();
        }
        Database::Postgres { pool, .. } => {
            sqlx::query("DELETE FROM jobs").execute(pool).await.unwrap();
        }
    }
}

fn checkpoint(workspace: &Workspace) -> Value {
    json!({"reconciled_generation": workspace.generation, "reconciled_at": 100, "retained": "checkpoint"})
}

async fn enqueue(
    database: &Database,
    workspace: &Workspace,
    payload: Value,
    available_at: i64,
) -> Uuid {
    database
        .enqueue_job(
            NewJob {
                kind: "reconcile_workspace".to_owned(),
                workspace_id: Some(workspace.id),
                payload,
                available_at,
            },
            100,
        )
        .await
        .unwrap()
}

async fn snapshot(database: &Database, id: Uuid) -> Value {
    macro_rules! read {
        ($pool:expr) => {{
            let row = sqlx::query("SELECT kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at FROM jobs WHERE id = $1")
                .bind(id.to_string()).fetch_one($pool).await.unwrap();
            json!({
                "kind": row.get::<String, _>("kind"),
                "workspace_id": row.get::<Option<String>, _>("workspace_id"),
                "payload_json": row.get::<String, _>("payload_json"),
                "status": row.get::<String, _>("status"),
                "available_at": row.get::<i64, _>("available_at"),
                "lease_owner": row.get::<Option<String>, _>("lease_owner"),
                "lease_expires_at": row.get::<Option<i64>, _>("lease_expires_at"),
                "attempts": row.get::<i64, _>("attempts"),
                "created_at": row.get::<i64, _>("created_at"),
                "updated_at": row.get::<i64, _>("updated_at"),
            })
        }};
    }
    match database {
        Database::Sqlite { pool, .. } => read!(pool),
        Database::Postgres { pool, .. } => read!(pool),
    }
}

fn other_installation(database: &Database) -> Database {
    let installation_id = "parking-other".parse().unwrap();
    match database {
        Database::Sqlite { pool, .. } => Database::Sqlite {
            pool: pool.clone(),
            installation_id,
        },
        Database::Postgres { pool, .. } => Database::Postgres {
            pool: pool.clone(),
            installation_id,
        },
    }
}

async fn claim(database: &Database, expected: Uuid, owner: &str, now: i64) {
    let claimed = database
        .claim_job(owner, now, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, expected);
}

async fn set_state(database: &Database, workspace: &Workspace, state: &str, generation: u64) {
    let generation = i64::try_from(generation).unwrap();
    match database {
        Database::Sqlite { pool, .. } => {
            sqlx::query("UPDATE workspaces SET state = $1, generation = $2 WHERE id = $3")
                .bind(state)
                .bind(generation)
                .bind(workspace.id.to_string())
                .execute(pool)
                .await
                .unwrap();
        }
        Database::Postgres { pool, .. } => {
            sqlx::query("UPDATE workspaces SET state = $1, generation = $2 WHERE id = $3")
                .bind(state)
                .bind(generation)
                .bind(workspace.id.to_string())
                .execute(pool)
                .await
                .unwrap();
        }
    }
}

async fn exercise(database: &Database, url: &str) {
    let workspace = seed(database).await;
    let id = enqueue(database, &workspace, checkpoint(&workspace), 100).await;
    claim(database, id, "worker-a", 100).await;
    let before = snapshot(database, id).await;
    assert!(matches!(
        database
            .park_workspace_job(id, "worker-b", &checkpoint(&workspace), 160, 101)
            .await,
        Err(StorageError::LeaseNotOwned(_))
    ));
    assert_eq!(snapshot(database, id).await, before);
    let foreign = other_installation(database);
    assert!(matches!(
        foreign
            .park_workspace_job(id, "worker-a", &checkpoint(&workspace), 160, 101)
            .await,
        Err(StorageError::LeaseNotOwned(_))
    ));
    assert_eq!(snapshot(database, id).await, before);

    let stored = database
        .park_workspace_job(id, "worker-a", &checkpoint(&workspace), 160, 101)
        .await
        .unwrap();
    assert_eq!(stored["runtime_wait"]["until"], 160);
    Uuid::parse_str(stored["runtime_wait"]["nonce"].as_str().unwrap()).unwrap();
    let parked_row = snapshot(database, id).await;
    assert_eq!(
        serde_json::from_str::<Value>(parked_row["payload_json"].as_str().unwrap()).unwrap(),
        stored
    );
    assert_eq!(parked_row["attempts"], 0);
    assert_eq!(parked_row["status"], "pending");
    assert!(parked_row["lease_owner"].is_null());
    assert!(parked_row["lease_expires_at"].is_null());

    // Reconstruct from a new database connection, without any worker/cache memory.
    let observer = Database::connect(url, database.installation_id().clone())
        .await
        .unwrap();
    let old = observer
        .get_parked_workspace_job(id, 102)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        old.payload_json,
        parked_row["payload_json"].as_str().unwrap()
    );
    assert_eq!(
        (old.generation, old.reconciled_at, old.desired_replicas),
        (workspace.generation, 100, 1)
    );
    assert_eq!(
        observer
            .list_parked_workspace_jobs(None, 10, 102)
            .await
            .unwrap()
            .jobs,
        vec![old.clone()]
    );
    assert!(
        foreign
            .get_parked_workspace_job(id, 102)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        foreign
            .list_parked_workspace_jobs(None, 10, 102)
            .await
            .unwrap()
            .jobs
            .is_empty()
    );
    assert!(!foreign.wake_parked_workspace_job(&old, 103).await.unwrap());
    let mut equivalent_payload = old.clone();
    equivalent_payload.payload_json = serde_json::to_string_pretty(&stored).unwrap();
    assert!(
        !observer
            .wake_parked_workspace_job(&equivalent_payload, 103)
            .await
            .unwrap(),
        "CAS compares the stored text, not semantic JSON equality"
    );
    assert_eq!(snapshot(database, id).await, parked_row);

    assert!(
        database
            .try_acquire_workspace_lease(
                workspace.id,
                "another-runtime-task",
                102,
                Duration::from_secs(200)
            )
            .await
            .unwrap()
    );
    assert!(observer.wake_parked_workspace_job(&old, 103).await.unwrap());
    let mut expected = parked_row;
    expected["available_at"] = json!(103);
    expected["updated_at"] = json!(103);
    assert_eq!(
        snapshot(database, id).await,
        expected,
        "wake may only update due time and updated_at"
    );
    assert!(
        !database
            .try_acquire_workspace_lease(workspace.id, "observer", 104, Duration::from_secs(30))
            .await
            .unwrap()
    );
    assert!(!observer.wake_parked_workspace_job(&old, 104).await.unwrap());
    claim(database, id, "worker-b", 104).await;
    assert!(
        observer
            .get_parked_workspace_job(id, 105)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        !observer.wake_parked_workspace_job(&old, 105).await.unwrap(),
        "running rows must not be woken"
    );
    let renewed = database
        .park_workspace_job(id, "worker-b", &checkpoint(&workspace), 160, 105)
        .await
        .unwrap();
    assert_ne!(
        renewed["runtime_wait"]["nonce"],
        stored["runtime_wait"]["nonce"]
    );
    assert!(
        !observer.wake_parked_workspace_job(&old, 106).await.unwrap(),
        "same deadline with a new nonce rejects a late wake"
    );
    let current = observer
        .get_parked_workspace_job(id, 106)
        .await
        .unwrap()
        .unwrap();
    assert!(
        !observer
            .wake_parked_workspace_job(&current, 160)
            .await
            .unwrap(),
        "expired cycles cannot be woken"
    );
    assert!(
        observer
            .wake_parked_workspace_job(&current, 107)
            .await
            .unwrap()
    );

    // Real failures retain their attempt; ordinary Pending / lease contention refund it.
    for refund in [false, true] {
        clear_jobs(database).await;
        let id = enqueue(database, &workspace, checkpoint(&workspace), 100).await;
        claim(database, id, "worker-a", 100).await;
        database
            .park_workspace_job(id, "worker-a", &checkpoint(&workspace), 160, 101)
            .await
            .unwrap();
        let old = observer
            .get_parked_workspace_job(id, 102)
            .await
            .unwrap()
            .unwrap();
        assert!(observer.wake_parked_workspace_job(&old, 103).await.unwrap());
        claim(database, id, "worker-b", 104).await;
        if refund {
            assert!(
                !database
                    .try_acquire_workspace_lease(
                        workspace.id,
                        "worker-b",
                        104,
                        Duration::from_secs(30),
                    )
                    .await
                    .unwrap(),
                "the runtime lease is still held by another task"
            );
            database
                .defer_job_without_attempt(id, "worker-b", 160, 105)
                .await
                .unwrap();
        } else {
            database.defer_job(id, "worker-b", 160, 105).await.unwrap();
        }
        let after = snapshot(database, id).await;
        let payload: Value = serde_json::from_str(after["payload_json"].as_str().unwrap()).unwrap();
        assert!(payload.get("runtime_wait").is_none());
        assert_eq!(payload, checkpoint(&workspace));
        assert_eq!(after["attempts"], if refund { 0 } else { 1 });
        assert_eq!(after["available_at"], 160);
        assert!(
            observer
                .get_parked_workspace_job(id, 106)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            observer
                .list_parked_workspace_jobs(None, 10, 106)
                .await
                .unwrap()
                .jobs
                .is_empty()
        );
        assert!(!observer.wake_parked_workspace_job(&old, 106).await.unwrap());
        assert_eq!(snapshot(database, id).await, after);
        if !refund {
            claim(database, id, "worker-c", 160).await;
            database
                .park_workspace_job(id, "worker-c", &checkpoint(&workspace), 220, 161)
                .await
                .unwrap();
            assert_eq!(
                snapshot(database, id).await["attempts"],
                1,
                "parking refunds only the current claim, preserving real failures"
            );
        }
    }

    clear_jobs(database).await;
    // Malformed rows consume raw pages, but must not hide subsequent valid parks.
    let valid_marker = json!({"nonce": Uuid::now_v7(), "until": 160});
    let mut ids = Vec::new();
    for sequence in 1..=5 {
        let mut payload = checkpoint(&workspace);
        payload["runtime_wait"] = match sequence {
            1 => json!({"nonce": "not-a-uuid", "until": 160}),
            3 => json!({"nonce": Uuid::now_v7(), "until": 159}),
            _ => valid_marker.clone(),
        };
        if sequence == 5 {
            payload["reconciled_generation"] = json!(workspace.generation + 1);
        }
        let original = enqueue(database, &workspace, payload, 160).await;
        let id = Uuid::from_u128(sequence);
        match database {
            Database::Sqlite { pool, .. } => {
                sqlx::query("UPDATE jobs SET id = $1 WHERE id = $2")
                    .bind(id.to_string())
                    .bind(original.to_string())
                    .execute(pool)
                    .await
                    .unwrap();
            }
            Database::Postgres { pool, .. } => {
                sqlx::query("UPDATE jobs SET id = $1 WHERE id = $2")
                    .bind(id.to_string())
                    .bind(original.to_string())
                    .execute(pool)
                    .await
                    .unwrap();
            }
        }
        ids.push(id);
    }
    let first = observer
        .list_parked_workspace_jobs(None, 1, 110)
        .await
        .unwrap();
    assert!(first.jobs.is_empty());
    assert_eq!(first.next, Some(ids[0]));
    let mut after = first.next;
    let mut found = Vec::new();
    for _ in 0..5 {
        let page = observer
            .list_parked_workspace_jobs(after, 1, 110)
            .await
            .unwrap();
        assert!(page.jobs.len() <= 1);
        found.extend(page.jobs.into_iter().map(|job| job.id));
        after = page.next;
        if after.is_none() {
            break;
        }
    }
    assert_eq!(found, vec![ids[1], ids[3]]);
    assert!(after.is_none());
    assert!(
        observer
            .get_parked_workspace_job(ids[4], 110)
            .await
            .unwrap()
            .is_none()
    );
    set_state(database, &workspace, "stopping", workspace.generation).await;
    assert_eq!(
        observer
            .get_parked_workspace_job(ids[1], 110)
            .await
            .unwrap()
            .unwrap()
            .desired_replicas,
        0
    );
    set_state(database, &workspace, "ready", workspace.generation).await;
    assert!(
        observer
            .list_parked_workspace_jobs(None, 10, 110)
            .await
            .unwrap()
            .jobs
            .is_empty()
    );
    set_state(database, &workspace, "starting", workspace.generation + 1).await;
    let current = observer
        .list_parked_workspace_jobs(None, 10, 110)
        .await
        .unwrap();
    assert_eq!(
        current.jobs.iter().map(|job| job.id).collect::<Vec<_>>(),
        vec![ids[4]]
    );
    assert!(
        observer
            .list_parked_workspace_jobs(None, 10, 160)
            .await
            .unwrap()
            .jobs
            .is_empty()
    );
}

#[tokio::test]
async fn sqlite_parks_survive_reconnect_and_are_woken_only_by_exact_identity() {
    let path = std::env::temp_dir().join(format!("mwc-runtime-parking-{}.sqlite", Uuid::now_v7()));
    let url = format!("sqlite://{}", path.display());
    let database = Database::connect(&url, "parking-ci".parse().unwrap())
        .await
        .unwrap();
    exercise(&database, &url).await;
    if let Database::Sqlite { pool, .. } = &database {
        pool.close().await;
    }
    drop(database);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

#[tokio::test]
async fn postgres_parks_survive_reconnect_and_are_woken_only_by_exact_identity() {
    let Ok(url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        eprintln!("skipping PostgreSQL runtime parking test: MWC_TEST_POSTGRES_URL is not set");
        return;
    };
    let schema = format!("mwc_runtime_parking_{}", Uuid::now_v7().simple());
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
    let installation: InstallationId = "parking-ci".parse().unwrap();
    let database = Database::connect(scoped.as_str(), installation)
        .await
        .unwrap();
    exercise(&database, scoped.as_str()).await;
    if let Database::Postgres { pool, .. } = &database {
        let workspace_id: String = sqlx::query_scalar("SELECT id FROM workspaces LIMIT 1")
            .fetch_one(pool)
            .await
            .unwrap();
        let workspace = database
            .get_workspace(Uuid::parse_str(&workspace_id).unwrap())
            .await
            .unwrap();
        for refresh_nonce in [true, false] {
            clear_jobs(&database).await;
            let id = enqueue(&database, &workspace, checkpoint(&workspace), 100).await;
            claim(&database, id, "worker-a", 100).await;
            let mut payload = database
                .park_workspace_job(id, "worker-a", &checkpoint(&workspace), 160, 101)
                .await
                .unwrap();
            let old = database
                .get_parked_workspace_job(id, 102)
                .await
                .unwrap()
                .unwrap();

            let mut transaction = pool.begin().await.unwrap();
            let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *transaction)
                .await
                .unwrap();
            sqlx::query("SELECT id FROM jobs WHERE id = $1 FOR UPDATE")
                .bind(id.to_string())
                .fetch_one(&mut *transaction)
                .await
                .unwrap();
            let waking_database = database.clone();
            let stale = old.clone();
            let wake = tokio::spawn(async move {
                waking_database.wake_parked_workspace_job(&stale, 103).await
            });
            // Prove the independent wake UPDATE reached this row's lock before
            // changing it, so PostgreSQL must recheck its WHERE after commit.
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let waiting: bool = sqlx::query_scalar(
                        "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock' AND $1 = ANY(pg_blocking_pids(pid)) AND query LIKE 'UPDATE jobs SET available_at = %')",
                    )
                    .bind(blocker_pid)
                    .fetch_one(pool)
                    .await
                    .unwrap();
                    if waiting {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("old wake must wait on the explicitly locked job row");

            // Model claim followed by either a new park or a real failure defer
            // in the lock holder; both invalidate the already-waiting wake.
            let claimed = sqlx::query("UPDATE jobs SET status = 'running', lease_owner = 'worker-b', lease_expires_at = 135, attempts = attempts + 1 WHERE id = $1 AND status = 'pending'")
                .bind(id.to_string())
                .execute(&mut *transaction)
                .await
                .unwrap();
            assert_eq!(claimed.rows_affected(), 1);
            let due = if refresh_nonce {
                payload["runtime_wait"]["nonce"] = json!(Uuid::now_v7());
                160_i64
            } else {
                payload.as_object_mut().unwrap().remove("runtime_wait");
                400_i64
            };
            let deferred = sqlx::query("UPDATE jobs SET status = 'pending', payload_json = $1, available_at = $2, updated_at = 105, lease_owner = NULL, lease_expires_at = NULL, attempts = attempts - $3 WHERE id = $4 AND status = 'running' AND lease_owner = 'worker-b'")
                .bind(serde_json::to_string(&payload).unwrap())
                .bind(due)
                .bind(i64::from(refresh_nonce))
                .bind(id.to_string())
                .execute(&mut *transaction)
                .await
                .unwrap();
            assert_eq!(deferred.rows_affected(), 1);
            transaction.commit().await.unwrap();

            assert!(
                !tokio::time::timeout(Duration::from_secs(5), wake)
                    .await
                    .expect("blocked old wake must finish after commit")
                    .unwrap()
                    .unwrap()
            );
            let after = snapshot(&database, id).await;
            assert_eq!(after["available_at"], due);
            assert_eq!(after["updated_at"], 105);
            assert_eq!(after["attempts"], if refresh_nonce { 0 } else { 1 });
            assert_eq!(after["status"], "pending");
            assert_eq!(after["lease_owner"], Value::Null);
            assert_eq!(after["lease_expires_at"], Value::Null);
            assert_eq!(
                serde_json::from_str::<Value>(after["payload_json"].as_str().unwrap()).unwrap(),
                payload
            );
            let fresh = database.get_parked_workspace_job(id, 106).await.unwrap();
            if refresh_nonce {
                let fresh = fresh.unwrap();
                assert_ne!(fresh.payload_json, old.payload_json);
                assert!(
                    database
                        .wake_parked_workspace_job(&fresh, 106)
                        .await
                        .unwrap()
                );
            } else {
                assert!(fresh.is_none());
                assert!(!database.wake_parked_workspace_job(&old, 106).await.unwrap());
                assert_eq!(snapshot(&database, id).await, after);
            }
        }
        pool.close().await;
    }
    drop(database);
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
}
