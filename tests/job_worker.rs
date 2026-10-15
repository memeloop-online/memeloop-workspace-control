use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use memeloop_workspace_control::{
    auth::ApiKeyScope,
    jobs::{JobHandler, JobHandlerError, JobWorker},
    quota::Resources,
    storage::{
        ClaimedJob, CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate, Database, NewJob,
        StorageError,
    },
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::{AccessMode, WorkspaceState},
};
use serde_json::json;

async fn database() -> Database {
    let database = Database::connect("sqlite::memory:", "worker-test".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
}

async fn workspace_database() -> (Database, uuid::Uuid, uuid::Uuid, u64) {
    let database = database().await;
    let image = "registry.example/workspace:1";
    database.upsert_image_policy(image, true, 1).await.unwrap();
    let admin = database
        .create_user_with_initial_key(
            "Admin",
            "terminal-failure-admin-000000000000000000000000",
            true,
            ApiKeyScope::initial_key_defaults(true),
            31_536_000,
            1,
        )
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Terminal failure".to_owned(),
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
                    "Terminal failure",
                    WorkspaceTemplateSpec::standard(
                        image,
                        AccessMode::Internal,
                        Resources {
                            cpu_millis: 1_000,
                            memory_mib: 2_048,
                            gpu_count: 0,
                            disk_gib: 20,
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
                name: "Terminal failure workspace".to_owned(),
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
    (
        database,
        organization.id,
        workspace.id,
        workspace.generation,
    )
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

struct PendingThenSuccessHandler {
    calls: AtomicUsize,
    pending_calls: usize,
}

impl JobHandler for PendingThenSuccessHandler {
    async fn handle(&self, _job: &ClaimedJob) -> Result<(), JobHandlerError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call < self.pending_calls {
            Err(JobHandlerError::Pending("waiting for readiness".to_owned()))
        } else {
            Ok(())
        }
    }
}

struct FailedPendingSuccessHandler {
    calls: AtomicUsize,
}

impl JobHandler for FailedPendingSuccessHandler {
    async fn handle(&self, _job: &ClaimedJob) -> Result<(), JobHandlerError> {
        match self.calls.fetch_add(1, Ordering::SeqCst) {
            0 => Err(JobHandlerError::Failed("temporary failure".to_owned())),
            1 => Err(JobHandlerError::Pending("waiting for readiness".to_owned())),
            _ => Ok(()),
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

#[tokio::test]
async fn pending_worker_execution_can_wait_past_retry_limit_then_complete_once() {
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
    let handler = Arc::new(PendingThenSuccessHandler {
        calls: AtomicUsize::new(0),
        pending_calls: 12,
    });
    let worker = JobWorker::new(database.clone(), handler.clone(), "replica-a".to_owned());

    for pending in 0..12 {
        assert!(worker.run_once(100 + pending * 5).await.unwrap());
    }
    assert!(worker.run_once(160).await.unwrap());
    assert!(!worker.run_once(161).await.unwrap());
    assert_eq!(handler.calls.load(Ordering::SeqCst), 13);

    let (status, attempts): (String, i64) =
        sqlx::query_as("SELECT status, attempts FROM jobs WHERE id = ?1")
            .bind(job_id.to_string())
            .fetch_one(match &database {
                Database::Sqlite { pool, .. } => pool,
                Database::Postgres { .. } => unreachable!(),
            })
            .await
            .unwrap();
    assert_eq!(status, "completed");
    assert_eq!(attempts, 1);
}

#[tokio::test]
async fn pending_wait_restores_only_its_claim_after_a_real_failure() {
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
    let handler = Arc::new(FailedPendingSuccessHandler {
        calls: AtomicUsize::new(0),
    });
    let worker = JobWorker::new(database.clone(), handler.clone(), "replica-a".to_owned());

    assert!(worker.run_once(100).await.unwrap());
    assert!(worker.run_once(102).await.unwrap());

    let (status, attempts): (String, i64) =
        sqlx::query_as("SELECT status, attempts FROM jobs WHERE id = ?1")
            .bind(job_id.to_string())
            .fetch_one(match &database {
                Database::Sqlite { pool, .. } => pool,
                Database::Postgres { .. } => unreachable!(),
            })
            .await
            .unwrap();
    assert_eq!(status, "pending");
    assert_eq!(attempts, 1);

    assert!(worker.run_once(107).await.unwrap());
    assert_eq!(handler.calls.load(Ordering::SeqCst), 3);
    let (status, attempts): (String, i64) =
        sqlx::query_as("SELECT status, attempts FROM jobs WHERE id = ?1")
            .bind(job_id.to_string())
            .fetch_one(match &database {
                Database::Sqlite { pool, .. } => pool,
                Database::Postgres { .. } => unreachable!(),
            })
            .await
            .unwrap();
    assert_eq!(status, "completed");
    assert_eq!(attempts, 2);
}

#[tokio::test]
async fn pending_defer_rejects_a_wrong_lease_owner() {
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
    let claimed = database
        .claim_job("owner-a", 100, Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, job_id);
    assert_eq!(claimed.attempts, 1);

    let result = database
        .defer_job_without_attempt(job_id, "owner-b", 105, 100)
        .await;
    assert!(matches!(
        result,
        Err(StorageError::LeaseNotOwned(id)) if id == job_id
    ));

    let (status, attempts, owner): (String, i64, Option<String>) =
        sqlx::query_as("SELECT status, attempts, lease_owner FROM jobs WHERE id = ?1")
            .bind(job_id.to_string())
            .fetch_one(match &database {
                Database::Sqlite { pool, .. } => pool,
                Database::Postgres { .. } => unreachable!(),
            })
            .await
            .unwrap();
    assert_eq!(status, "running");
    assert_eq!(attempts, 1);
    assert_eq!(owner.as_deref(), Some("owner-a"));
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

#[tokio::test]
async fn terminal_workspace_failure_marks_matching_generation_failed() {
    let (database, organization_id, workspace_id, generation) = workspace_database().await;

    assert!(
        database
            .mark_workspace_failed_if_generation(workspace_id, generation, 5)
            .await
            .unwrap()
    );
    assert_eq!(
        database.get_workspace(workspace_id).await.unwrap().state,
        WorkspaceState::Failed
    );
    let events = database
        .list_events(organization_id, Some(workspace_id), 20)
        .await
        .unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].payload["action"], "mark_failed");
}

#[tokio::test]
async fn terminal_workspace_failure_ignores_stale_generation() {
    let (database, organization_id, workspace_id, generation) = workspace_database().await;

    assert!(
        !database
            .mark_workspace_failed_if_generation(workspace_id, generation + 1, 5)
            .await
            .unwrap()
    );
    assert_eq!(
        database.get_workspace(workspace_id).await.unwrap().state,
        WorkspaceState::Provisioning
    );
    assert_eq!(
        database
            .list_events(organization_id, Some(workspace_id), 20)
            .await
            .unwrap()
            .len(),
        1
    );
}

struct CheckpointHandler;

impl JobHandler for CheckpointHandler {
    async fn handle(&self, job: &ClaimedJob) -> Result<(), JobHandlerError> {
        assert_eq!(job.payload["generation"], 17);
        if job.payload["reconciled_generation"] == 17 {
            assert_eq!(job.attempts, 1);
            return Ok(());
        }
        let mut payload = job.payload.clone();
        payload["reconciled_generation"] = 17.into();
        Err(JobHandlerError::PendingWithCheckpoint {
            message: "waiting for readiness".to_owned(),
            payload,
        })
    }
}

#[tokio::test]
async fn readiness_checkpoint_survives_worker_restart_and_retries_after_one_second() {
    let database = database().await;
    database
        .enqueue_job(
            NewJob {
                kind: "test".to_owned(),
                workspace_id: None,
                payload: json!({"generation":17}),
                available_at: 100,
            },
            100,
        )
        .await
        .unwrap();
    let worker = JobWorker::new(
        database.clone(),
        Arc::new(CheckpointHandler),
        "owner-a".to_owned(),
    );
    assert!(worker.run_once(100).await.unwrap());
    assert!(!worker.run_once(100).await.unwrap());
    let restarted = JobWorker::new(database, Arc::new(CheckpointHandler), "owner-b".to_owned());
    assert!(restarted.run_once(101).await.unwrap());
    assert!(!restarted.run_once(102).await.unwrap());
}

#[tokio::test]
async fn checkpoint_rejects_a_wrong_lease_owner_without_changing_payload() {
    let database = database().await;
    let id = database
        .enqueue_job(
            NewJob {
                kind: "test".to_owned(),
                workspace_id: None,
                payload: json!({"generation":17}),
                available_at: 100,
            },
            100,
        )
        .await
        .unwrap();
    database
        .claim_job("owner-a", 100, Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    let result = database
        .save_job_checkpoint(id, "owner-b", &json!({"generation":18}), 100)
        .await;
    assert!(matches!(result, Err(StorageError::LeaseNotOwned(actual)) if actual == id));
    database
        .defer_job_without_attempt(id, "owner-a", 101, 100)
        .await
        .unwrap();
    let job = database
        .claim_job("owner-c", 101, Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.payload, json!({"generation":17}));
}

struct PoolBarrierHandler {
    calls: AtomicUsize,
    barrier: tokio::sync::Barrier,
}

impl JobHandler for PoolBarrierHandler {
    async fn handle(&self, _job: &ClaimedJob) -> Result<(), JobHandlerError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.barrier.wait().await;
        Ok(())
    }
}

#[tokio::test]
async fn worker_pool_runs_four_jobs_and_drains_in_flight_work_on_shutdown() {
    let database = database().await;
    database.set_job_worker_concurrency(4, 100).await.unwrap();
    for _ in 0..5 {
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
    }
    let handler = Arc::new(PoolBarrierHandler {
        calls: AtomicUsize::new(0),
        barrier: tokio::sync::Barrier::new(5),
    });
    let worker = JobWorker::new(database.clone(), handler.clone(), "pool".to_owned());
    let (shutdown, receiver) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(async move { worker.run_pool_until_shutdown(receiver).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        while handler.calls.load(Ordering::SeqCst) < 4 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // Stop admitting new jobs before allowing the four in-flight jobs to finish.
    shutdown.send(true).unwrap();
    handler.barrier.wait().await;
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(handler.calls.load(Ordering::SeqCst), 4);
    let counts = database.job_counts().await.unwrap();
    assert_eq!(counts.completed, 4);
    assert_eq!(counts.pending, 1);
    assert_eq!(counts.running, 0);
}

struct LeaseBlockingHandler {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    calls: AtomicUsize,
}

impl JobHandler for LeaseBlockingHandler {
    async fn handle(&self, _job: &ClaimedJob) -> Result<(), JobHandlerError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        self.release.notified().await;
        Ok(())
    }
}

#[tokio::test]
async fn distinct_worker_owners_cannot_execute_the_same_workspace_concurrently() {
    let (database, _, workspace_id, _) = workspace_database().await;
    let pool = match &database {
        Database::Sqlite { pool, .. } => pool,
        Database::Postgres { .. } => unreachable!(),
    };
    sqlx::query("DELETE FROM jobs").execute(pool).await.unwrap();
    for _ in 0..2 {
        database
            .enqueue_job(
                NewJob {
                    kind: "test".to_owned(),
                    workspace_id: Some(workspace_id),
                    payload: json!({}),
                    available_at: 100,
                },
                100,
            )
            .await
            .unwrap();
    }
    let handler = Arc::new(LeaseBlockingHandler {
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
        calls: AtomicUsize::new(0),
    });
    let first = JobWorker::new(
        database.clone(),
        handler.clone(),
        "pool:worker-0".to_owned(),
    );
    let second = JobWorker::new(
        database.clone(),
        handler.clone(),
        "pool:worker-1".to_owned(),
    );
    let task = tokio::spawn(async move { first.run_once(100).await });
    tokio::time::timeout(Duration::from_secs(5), handler.entered.notified())
        .await
        .unwrap();
    assert!(second.run_once(100).await.unwrap());
    assert_eq!(handler.calls.load(Ordering::SeqCst), 1);
    let counts = database.job_counts().await.unwrap();
    assert_eq!(counts.running, 1);
    assert_eq!(counts.pending, 1);
    handler.release.notify_one();
    assert!(task.await.unwrap().unwrap());
    assert_eq!(database.job_counts().await.unwrap().completed, 1);
}

#[tokio::test]
async fn worker_pool_admits_more_jobs_when_persisted_concurrency_changes() {
    let database = database().await;
    assert_eq!(database.job_worker_concurrency().await.unwrap(), 32);
    database.set_job_worker_concurrency(1, 100).await.unwrap();
    for _ in 0..2 {
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
    }
    let handler = Arc::new(PoolBarrierHandler {
        calls: AtomicUsize::new(0),
        barrier: tokio::sync::Barrier::new(3),
    });
    let worker = JobWorker::new(database.clone(), handler.clone(), "dynamic".to_owned());
    let (shutdown, receiver) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(async move { worker.run_pool_until_shutdown(receiver).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        while handler.calls.load(Ordering::SeqCst) < 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    database.set_job_worker_concurrency(2, 101).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while handler.calls.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    shutdown.send(true).unwrap();
    handler.barrier.wait().await;
    task.await.unwrap().unwrap();
    assert_eq!(database.job_counts().await.unwrap().completed, 2);
    for invalid in [0, 257] {
        assert!(matches!(
            database.set_job_worker_concurrency(invalid, 102).await,
            Err(StorageError::InvalidJobWorkerConcurrency)
        ));
    }
    assert_eq!(database.job_worker_concurrency().await.unwrap(), 2);
}

#[derive(Clone, Copy)]
enum SlowOutcome {
    Retry,
    Pending,
    Checkpoint,
    Success,
}

struct SlowHandler(SlowOutcome);

impl JobHandler for SlowHandler {
    async fn handle(&self, _job: &ClaimedJob) -> Result<(), JobHandlerError> {
        // Longer than the initial retry and checkpoint delays: using the claim
        // timestamp would make both deadlines expire before this call returns.
        tokio::time::sleep(Duration::from_secs(3)).await;
        match self.0 {
            SlowOutcome::Retry => Err(JobHandlerError::Failed("transient".to_owned())),
            SlowOutcome::Pending => Err(JobHandlerError::Pending("waiting".to_owned())),
            SlowOutcome::Checkpoint => Err(JobHandlerError::PendingWithCheckpoint {
                message: "applied".to_owned(),
                payload: json!({"stage":"applied"}),
            }),
            SlowOutcome::Success => Ok(()),
        }
    }
}

async fn assert_slow_job_deadline(outcome: SlowOutcome, delay: Option<i64>) {
    let database = database().await;
    let id = database
        .enqueue_job(
            NewJob {
                kind: "slow".to_owned(),
                workspace_id: None,
                payload: json!({}),
                available_at: 100,
            },
            100,
        )
        .await
        .unwrap();
    let worker = JobWorker::new(
        database.clone(),
        Arc::new(SlowHandler(outcome)),
        "slow-owner".to_owned(),
    );
    assert!(worker.run_once(100).await.unwrap());
    let pool = match &database {
        Database::Sqlite { pool, .. } => pool,
        Database::Postgres { .. } => unreachable!(),
    };
    let (updated_at, available_at, status, attempts): (i64, i64, String, i64) =
        sqlx::query_as("SELECT updated_at, available_at, status, attempts FROM jobs WHERE id = ?1")
            .bind(id.to_string())
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(
        updated_at >= 103,
        "completion time must include external work"
    );
    if let Some(delay) = delay {
        assert_eq!(status, "pending");
        assert_eq!(available_at, updated_at + delay);
        assert!(
            database
                .claim_job("probe", available_at - 1, Duration::from_secs(60))
                .await
                .unwrap()
                .is_none()
        );
        let retried = database
            .claim_job("probe", available_at, Duration::from_secs(60))
            .await
            .unwrap()
            .unwrap();
        if matches!(outcome, SlowOutcome::Checkpoint) {
            assert_eq!(retried.payload, json!({"stage":"applied"}));
        }
        assert_eq!(attempts, i64::from(matches!(outcome, SlowOutcome::Retry)));
    } else {
        assert_eq!(status, "completed");
    }
}

#[tokio::test]
async fn slow_failure_receives_full_backoff_after_execution() {
    assert_slow_job_deadline(SlowOutcome::Retry, Some(2)).await;
}

#[tokio::test]
async fn slow_pending_receives_full_readiness_delay_after_execution() {
    assert_slow_job_deadline(SlowOutcome::Pending, Some(5)).await;
}

#[tokio::test]
async fn slow_checkpoint_survives_until_its_post_execution_deadline() {
    assert_slow_job_deadline(SlowOutcome::Checkpoint, Some(1)).await;
}

#[tokio::test]
async fn slow_success_records_completion_after_execution() {
    assert_slow_job_deadline(SlowOutcome::Success, None).await;
}
