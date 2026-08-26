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
            Err(JobHandlerError("temporary failure".to_owned()))
        } else {
            Ok(())
        }
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
