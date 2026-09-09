use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use memeloop_workspace_control::{
    jobs::{JobHandler, JobHandlerError, JobWorker},
    storage::{ClaimedJob, Database, NewJob},
};
use serde_json::json;

async fn database() -> Database {
    let database = Database::connect("sqlite::memory:", "worker-test".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
}

struct CountingHandler {
    calls: AtomicUsize,
    fail: bool,
}

impl JobHandler for CountingHandler {
    async fn handle(&self, _job: &ClaimedJob) -> Result<(), JobHandlerError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            Err(JobHandlerError::Failed("temporary failure".to_owned()))
        } else {
            Ok(())
        }
    }
}

struct PendingHandler {
    calls: AtomicUsize,
}

impl JobHandler for PendingHandler {
    async fn handle(&self, _job: &ClaimedJob) -> Result<(), JobHandlerError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(JobHandlerError::Pending("waiting for readiness".to_owned()))
    }
}

#[tokio::test]
async fn successful_worker_execution_completes_job_exactly_once() {
    let database = database().await;
    database
        .enqueue_job(
            NewJob {
                kind: "test".to_owned(),
                workspace_id: None,
                payload: json!({}),
                available_at: 100,
            },
            100,
        )
        .await
        .unwrap();
    let handler = Arc::new(CountingHandler {
        calls: AtomicUsize::new(0),
        fail: false,
    });
    let worker = JobWorker::new(database.clone(), handler.clone(), "replica-a".to_owned());

    assert!(worker.run_once(100).await.unwrap());
    assert!(!worker.run_once(101).await.unwrap());
    assert_eq!(handler.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn failed_worker_execution_is_deferred_with_bounded_retry() {
    let database = database().await;
    database
        .enqueue_job(
            NewJob {
                kind: "test".to_owned(),
                workspace_id: None,
                payload: json!({}),
                available_at: 100,
            },
            100,
        )
        .await
        .unwrap();
    let handler = Arc::new(CountingHandler {
        calls: AtomicUsize::new(0),
        fail: true,
    });
    let worker = JobWorker::new(database, handler.clone(), "replica-a".to_owned());

    assert!(worker.run_once(100).await.unwrap());
    assert!(!worker.run_once(101).await.unwrap());
    assert!(worker.run_once(102).await.unwrap());
    assert_eq!(handler.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn pending_worker_execution_is_deferred_without_consuming_retry_budget() {
    let database = database().await;
    let job_id = database
        .enqueue_job(
            NewJob {
                kind: "test".to_owned(),
                workspace_id: None,
                payload: json!({}),
                available_at: 100,
            },
            100,
        )
        .await
        .unwrap();
    let handler = Arc::new(PendingHandler {
        calls: AtomicUsize::new(0),
    });
    let worker = JobWorker::new(database.clone(), handler.clone(), "replica-a".to_owned());

    assert!(worker.run_once(100).await.unwrap());
    assert!(!worker.run_once(101).await.unwrap());
    assert!(worker.run_once(105).await.unwrap());
    assert_eq!(handler.calls.load(Ordering::SeqCst), 2);

    let (status, attempts, available_at): (String, i64, i64) =
        sqlx::query_as("SELECT status, attempts, available_at FROM jobs WHERE id = ?1")
            .bind(job_id.to_string())
            .fetch_one(match &database {
                Database::Sqlite { pool, .. } => pool,
                Database::Postgres { .. } => unreachable!(),
            })
            .await
            .unwrap();
    assert_eq!(status, "pending");
    assert_eq!(attempts, 0);
    assert_eq!(available_at, 110);
}

#[tokio::test]
async fn permanently_failing_job_reaches_failed_terminal_state() {
    let database = database().await;
    database
        .enqueue_job(
            NewJob {
                kind: "test".to_owned(),
                workspace_id: None,
                payload: json!({}),
                available_at: 100,
            },
            100,
        )
        .await
        .unwrap();
    let handler = Arc::new(CountingHandler {
        calls: AtomicUsize::new(0),
        fail: true,
    });
    let worker = JobWorker::new(database.clone(), handler.clone(), "replica-a".to_owned());

    for attempt in 0..10 {
        assert!(worker.run_once(100 + attempt * 1_000).await.unwrap());
    }
    assert!(!worker.run_once(20_000).await.unwrap());
    assert_eq!(handler.calls.load(Ordering::SeqCst), 10);
    let counts = database.job_counts().await.unwrap();
    assert_eq!(counts.pending, 0);
    assert_eq!(counts.running, 0);
    assert_eq!(counts.failed, 1);
}
