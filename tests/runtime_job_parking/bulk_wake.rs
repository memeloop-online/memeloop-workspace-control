use std::{collections::HashSet, io::Write, time::Instant};

use super::*;
use memeloop_workspace_control::storage::ParkedWorkspaceJob;

async fn parked_batch(
    database: &Database,
    workspace: &Workspace,
    count: usize,
) -> Vec<ParkedWorkspaceJob> {
    let mut jobs = Vec::with_capacity(count);
    for _ in 0..count {
        let mut payload = checkpoint(workspace);
        payload["runtime_wait"] = json!({"nonce": Uuid::now_v7(), "until": 160});
        let id = enqueue(database, workspace, payload, 160).await;
        jobs.push(
            database
                .get_parked_workspace_job(id, 102)
                .await
                .unwrap()
                .unwrap(),
        );
    }
    jobs
}

async fn set_job(database: &Database, job: &ParkedWorkspaceJob, case: usize) {
    let mut payload: Value = serde_json::from_str(&job.payload_json).unwrap();
    payload["runtime_wait"]["nonce"] = json!(Uuid::now_v7());
    macro_rules! update {
        ($pool:expr) => {
            sqlx::query(
                "UPDATE jobs SET status = $1, payload_json = $2, installation_id = $3, \
                kind = $4, available_at = $5 WHERE id = $6",
            )
            .bind(if case == 2 { "running" } else { "pending" })
            .bind(if case == 3 {
                serde_json::to_string(&payload).unwrap()
            } else {
                job.payload_json.clone()
            })
            .bind(if case == 4 {
                "parking-other"
            } else {
                database.installation_id().as_str()
            })
            .bind(if case == 6 {
                "other_job_kind"
            } else {
                "reconcile_workspace"
            })
            .bind(if case == 5 { 103_i64 } else { 160 })
            .bind(job.id.to_string())
            .execute($pool)
            .await
            .unwrap()
        };
    }
    match database {
        Database::Sqlite { pool, .. } => {
            update!(pool);
        }
        Database::Postgres { pool, .. } => {
            update!(pool);
        }
    }
}

async fn mixed_batches(database: &Database) {
    let workspace = seed(database).await;
    assert!(
        database
            .wake_parked_workspace_jobs(&[], 103)
            .await
            .unwrap()
            .is_empty()
    );
    for count in [32, 256] {
        clear_jobs(database).await;
        let mut jobs = parked_batch(database, &workspace, count).await;
        let mut expected = HashSet::new();
        for (index, job) in jobs.iter_mut().enumerate() {
            let case = index % 9;
            set_job(database, job, case).await;
            match case {
                0 => {
                    expected.insert(job.id);
                }
                1 => {
                    let mut payload: Value = serde_json::from_str(&job.payload_json).unwrap();
                    payload["runtime_wait"]["nonce"] = json!(Uuid::now_v7());
                    job.payload_json = serde_json::to_string(&payload).unwrap();
                }
                5 => job.until = 103,
                8 => job.until = 159,
                7 => {
                    let payload: Value = serde_json::from_str(&job.payload_json).unwrap();
                    job.payload_json = serde_json::to_string_pretty(&payload).unwrap();
                }
                _ => {}
            }
        }
        let mut before = Vec::new();
        for job in &jobs {
            before.push(snapshot(database, job.id).await);
        }
        let local_candidates = jobs
            .iter()
            .filter(|job| expected.contains(&job.id))
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            other_installation(database)
                .wake_parked_workspace_jobs(&local_candidates, 103)
                .await
                .unwrap()
                .is_empty()
        );
        // A duplicate candidate must neither update twice nor inflate the receipt.
        jobs.push(jobs[0].clone());
        assert_eq!(
            database
                .wake_parked_workspace_jobs(&jobs, 103)
                .await
                .unwrap(),
            expected
        );
        for (job, mut original) in jobs.iter().zip(before) {
            if expected.contains(&job.id) {
                original["available_at"] = json!(103);
                original["updated_at"] = json!(103);
            }
            assert_eq!(snapshot(database, job.id).await, original, "{}", job.id);
        }
        assert!(
            database
                .wake_parked_workspace_jobs(&jobs, 104)
                .await
                .unwrap()
                .is_empty()
        );
    }
}

async fn bulk_3000(database: &Database, workspace: &Workspace, backend: &str) {
    clear_jobs(database).await;
    let jobs = parked_batch(database, workspace, 3_000).await;
    let expected = jobs.iter().map(|job| job.id).collect::<HashSet<_>>();
    let started = Instant::now();
    // This timeout bounds a stuck test; it is not a startup or throughput SLO.
    let updated = tokio::time::timeout(
        Duration::from_secs(10),
        database.wake_parked_workspace_jobs(&jobs, 103),
    )
    .await
    .expect("one 3000-job wake must finish within the test guard")
    .unwrap();
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
    assert_eq!(updated, expected);
    let sql = "SELECT COUNT(*) FROM jobs WHERE installation_id = $1 \
        AND status = 'pending' AND available_at = 103 AND updated_at = 103 AND attempts = 0";
    let count: i64 = match database {
        Database::Sqlite { pool, .. } => sqlx::query_scalar(sql)
            .bind(database.installation_id().as_str())
            .fetch_one(pool)
            .await
            .unwrap(),
        Database::Postgres { pool, .. } => sqlx::query_scalar(sql)
            .bind(database.installation_id().as_str())
            .fetch_one(pool)
            .await
            .unwrap(),
    };
    assert_eq!(count, 3_000);
    let replay = tokio::time::timeout(
        Duration::from_secs(10),
        database.wake_parked_workspace_jobs(&jobs, 104),
    )
    .await
    .expect("one 3000-job replay must finish within the test guard")
    .unwrap();
    assert!(replay.is_empty());
    // Direct stdout keeps this bounded aggregate visible in successful CI logs.
    writeln!(
        std::io::stdout().lock(),
        "MWC_BULK_WAKE_ACCEPTANCE {}",
        json!({
            "backend": backend,
            "batch": jobs.len(),
            "elapsed_ms": elapsed_ms,
            "updated": updated.len(),
            "persisted_due_pending_zero_attempts": count,
            "replayed_updated": replay.len(),
            "completed": true,
        })
    )
    .unwrap();
}

async fn sqlite_database() -> (Database, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("mwc-bulk-wake-{}.sqlite", Uuid::now_v7()));
    let database = Database::connect(
        &format!("sqlite://{}", path.display()),
        "parking-ci".parse().unwrap(),
    )
    .await
    .unwrap();
    (database, path)
}

async fn close_sqlite(database: Database, path: std::path::PathBuf) {
    if let Database::Sqlite { pool, .. } = &database {
        pool.close().await;
    }
    drop(database);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

#[tokio::test]
async fn sqlite_bulk_wake_preserves_exact_cas_for_32_and_256_mixed_parks() {
    let (database, path) = sqlite_database().await;
    mixed_batches(&database).await;
    close_sqlite(database, path).await;
}

#[tokio::test]
async fn sqlite_bulk_wake_3000_same_batch_returns_every_persisted_job_once() {
    let (database, path) = sqlite_database().await;
    let workspace = seed(&database).await;
    bulk_3000(&database, &workspace, "sqlite").await;
    close_sqlite(database, path).await;
}

#[tokio::test]
async fn sqlite_bulk_wake_is_atomic_and_retry_after_caller_cancellation_is_harmless() {
    let (database, path) = sqlite_database().await;
    let workspace = seed(&database).await;
    let jobs = parked_batch(&database, &workspace, 32).await;
    let Database::Sqlite { pool, .. } = &database else {
        unreachable!()
    };
    sqlx::query(
        "CREATE TRIGGER abort_bulk_wake BEFORE UPDATE OF available_at ON jobs \
        WHEN NEW.available_at = 103 AND OLD.id = (SELECT MAX(id) FROM jobs) \
        BEGIN SELECT RAISE(ABORT, 'batch interrupted'); END",
    )
    .execute(pool)
    .await
    .unwrap();
    assert!(
        database
            .wake_parked_workspace_jobs(&jobs, 103)
            .await
            .is_err()
    );
    for job in &jobs {
        assert_eq!(snapshot(&database, job.id).await["available_at"], 160);
    }
    sqlx::query("DROP TRIGGER abort_bulk_wake")
        .execute(pool)
        .await
        .unwrap();

    // Model cancellation after the SQL committed but before the caller consumed
    // its receipt. Cancellation cannot be treated as proof that no write happened.
    let (committed, receipt) = tokio::sync::oneshot::channel();
    let writer = database.clone();
    let batch = jobs.clone();
    let task = tokio::spawn(async move {
        let updated = writer
            .wake_parked_workspace_jobs(&batch, 103)
            .await
            .unwrap();
        committed.send(updated).unwrap();
        std::future::pending::<()>().await;
    });
    assert_eq!(
        receipt.await.unwrap(),
        jobs.iter().map(|job| job.id).collect::<HashSet<_>>()
    );
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(
        database
            .wake_parked_workspace_jobs(&jobs, 104)
            .await
            .unwrap()
            .is_empty()
    );
    for job in &jobs {
        let row = snapshot(&database, job.id).await;
        assert_eq!(row["available_at"], 103);
        assert_eq!(row["updated_at"], 103);
    }
    close_sqlite(database, path).await;
}

#[tokio::test]
async fn postgres_bulk_wake_preserves_cas_and_cancelled_statement_cannot_partially_commit() {
    let Ok(url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        eprintln!("skipping PostgreSQL bulk wake test: MWC_TEST_POSTGRES_URL is not set");
        return;
    };
    let schema = format!("mwc_bulk_wake_{}", Uuid::now_v7().simple());
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
    let database = Database::connect(scoped.as_str(), "parking-ci".parse().unwrap())
        .await
        .unwrap();
    mixed_batches(&database).await;
    clear_jobs(&database).await;
    let Database::Postgres { pool, .. } = &database else {
        unreachable!()
    };
    let workspace_id: String = sqlx::query_scalar("SELECT id FROM workspaces LIMIT 1")
        .fetch_one(pool)
        .await
        .unwrap();
    let workspace = database
        .get_workspace(Uuid::parse_str(&workspace_id).unwrap())
        .await
        .unwrap();
    bulk_3000(&database, &workspace, "postgres").await;
    clear_jobs(&database).await;
    let jobs = parked_batch(&database, &workspace, 32).await;
    let mut transaction = pool.begin().await.unwrap();
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *transaction)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM jobs ORDER BY id FOR UPDATE")
        .fetch_all(&mut *transaction)
        .await
        .unwrap();
    let writer = database.clone();
    let batch = jobs.clone();
    let task = tokio::spawn(async move { writer.wake_parked_workspace_jobs(&batch, 103).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname = current_database() \
                AND wait_event_type = 'Lock' AND $1 = ANY(pg_blocking_pids(pid)) \
                AND query LIKE 'UPDATE jobs SET available_at = %')",
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
    .expect("bulk wake must reach the explicitly locked rows");
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    transaction.commit().await.unwrap();
    // A cancelled SQL future may have committed or may not have run. Repeating
    // the exact CAS is safe in either case and may return only all or none here.
    let retried = database
        .wake_parked_workspace_jobs(&jobs, 103)
        .await
        .unwrap();
    assert!(retried.is_empty() || retried == jobs.iter().map(|job| job.id).collect::<HashSet<_>>());
    assert!(
        database
            .wake_parked_workspace_jobs(&jobs, 104)
            .await
            .unwrap()
            .is_empty()
    );
    for job in &jobs {
        let row = snapshot(&database, job.id).await;
        assert_eq!(row["available_at"], 103);
        assert_eq!(row["updated_at"], 103);
    }
    pool.close().await;
    drop(database);
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
}
