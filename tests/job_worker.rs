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
