use std::time::Duration;

use memeloop_workspace_control::storage::{Database, NewJob, StorageError};
use serde_json::Value;
use uuid::Uuid;

async fn database() -> Database {
    let database = Database::connect("sqlite::memory:", "test-a".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
}

#[tokio::test]
async fn job_is_claimed_once_until_lease_expires() {
    let database = database().await;
    let job_id = database
        .enqueue_job(
            NewJob {
                kind: "reconcile_workspace".to_owned(),
                workspace_id: Some(Uuid::now_v7()),
                payload: serde_json::json!({"generation": 1}),
                available_at: 100,
            },
            90,
        )
        .await
        .unwrap();

    let first = database
        .claim_job("replica-a", 100, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.id, job_id);
    assert_eq!(first.attempts, 1);
    assert!(
        database
            .claim_job("replica-b", 110, Duration::from_secs(30))
            .await
            .unwrap()
            .is_none()
    );

    let reclaimed = database
        .claim_job("replica-b", 131, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reclaimed.id, job_id);
    assert_eq!(reclaimed.attempts, 2);
}

#[tokio::test]
async fn only_lease_owner_can_complete_job() {
    let database = database().await;
    let job_id = database
        .enqueue_job(
            NewJob {
                kind: "delete_workspace".to_owned(),
                workspace_id: None,
                payload: Value::Null,
                available_at: 1,
            },
            1,
        )
        .await
        .unwrap();
    database
        .claim_job("replica-a", 1, Duration::from_secs(30))
        .await
        .unwrap();

    assert!(matches!(
        database.complete_job(job_id, "replica-b", 2).await,
        Err(StorageError::LeaseNotOwned(id)) if id == job_id
    ));
    database.complete_job(job_id, "replica-a", 2).await.unwrap();
    assert!(
        database
            .claim_job("replica-c", 100, Duration::from_secs(30))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn active_job_lease_can_be_renewed_before_it_expires() {
    let database = database().await;
    let job_id = database
        .enqueue_job(
            NewJob {
                kind: "reconcile_workspace".to_owned(),
                workspace_id: None,
                payload: Value::Null,
                available_at: 100,
            },
            100,
        )
        .await
        .unwrap();
    database
        .claim_job("replica-a", 100, Duration::from_secs(10))
        .await
        .unwrap()
        .unwrap();
    database
        .renew_job_lease(job_id, "replica-a", 105, Duration::from_secs(10))
        .await
        .unwrap();
    assert!(
        database
            .claim_job("replica-b", 111, Duration::from_secs(10))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        database
            .claim_job("replica-b", 116, Duration::from_secs(10))
            .await
            .unwrap()
            .unwrap()
            .id,
        job_id
    );
}

#[tokio::test]
async fn database_cannot_be_reused_by_another_installation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("control-plane.sqlite");
    let url = format!("sqlite://{}?mode=rwc", path.display());

    let first = Database::connect(&url, "install-a".parse().unwrap())
        .await
        .unwrap();
    first.migrate().await.unwrap();
    drop(first);

    let second = Database::connect(&url, "install-b".parse().unwrap())
        .await
        .unwrap();
    assert!(matches!(
        second.migrate().await,
        Err(StorageError::InstallationMismatch { stored, .. }) if stored == "install-a"
    ));
}

#[tokio::test]
async fn sqlite_file_recovers_authoritative_state_and_expired_jobs_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("recover.sqlite");
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let installation_id = "sqlite-recovery"
        .parse::<memeloop_workspace_control::config::InstallationId>()
        .unwrap();
    let first = Database::connect(&url, installation_id.clone())
        .await
        .unwrap();
    first.migrate().await.unwrap();
    let job_id = first
        .enqueue_job(
            NewJob {
                kind: "restart-recovery".to_owned(),
                workspace_id: None,
                payload: serde_json::json!({"durable": true}),
                available_at: 10,
            },
            10,
        )
        .await
        .unwrap();
    first
        .claim_job("before-restart", 10, Duration::from_secs(5))
        .await
        .unwrap()
        .unwrap();
    drop(first);

    let recovered = Database::connect(&url, installation_id).await.unwrap();
    recovered.migrate().await.unwrap();
    assert!(
        recovered
            .claim_job("after-restart", 15, Duration::from_secs(5))
            .await
            .unwrap()
            .is_none(),
        "a lease is still valid at its exact expiry boundary"
    );
    assert_eq!(
        recovered
            .claim_job("after-restart", 16, Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap()
            .id,
        job_id
    );
}

#[tokio::test]
async fn migrations_are_versioned_and_idempotent() {
    let database = Database::connect("sqlite::memory:", "migration-test".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    assert_eq!(database.schema_version().await.unwrap(), 11);
    database.migrate().await.unwrap();
    assert_eq!(database.schema_version().await.unwrap(), 11);
}

#[tokio::test]
async fn schema_ten_backfills_yaml_for_a_legacy_template_row() {
    let database = Database::connect("sqlite::memory:", "profile-migration".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let Database::Sqlite { pool, .. } = &database else {
        unreachable!();
    };
    let template_id = Uuid::now_v7().to_string();
    sqlx::query("INSERT INTO workspace_templates (id, installation_id, organization_id, name, runtime_profile, image, access_mode, cpu_millis, memory_mib, gpu_count, disk_gib, enabled, created_at, updated_at) VALUES (?1, 'profile-migration', NULL, 'Legacy Rust', 'coder_token_center_rust_dev', 'registry.example/rust:legacy', 'internal', 2000, 4096, 0, 40, 1, 1, 1)")
        .bind(&template_id)
        .execute(pool)
        .await
        .unwrap();
    database.migrate().await.unwrap();

    let template = database
        .get_workspace_template(Uuid::parse_str(&template_id).unwrap())
        .await
        .unwrap();
    assert_eq!(template.name, "Legacy Rust");
    assert_eq!(template.template.workspace_user, "rust-dev");
    assert_eq!(template.template.workspace_home, "/home/rust-dev");
    assert!(
        template
            .yaml
            .contains("apiVersion: workspace.memeloop.dev/v1")
    );
    assert!(!template.yaml.contains("runtimeProfile"));
    assert!(!template.yaml.contains("runtime_profile"));
    assert_eq!(database.schema_version().await.unwrap(), 11);
}

#[tokio::test]
async fn workspace_lease_serializes_distinct_jobs_for_the_same_workspace() {
    let database = database().await;
    let workspace_id = Uuid::now_v7();
    assert!(
        database
            .try_acquire_workspace_lease(workspace_id, "replica-a", 100, Duration::from_secs(30))
            .await
            .unwrap()
    );
    assert!(
        !database
            .try_acquire_workspace_lease(workspace_id, "replica-b", 110, Duration::from_secs(30))
            .await
            .unwrap()
    );
    assert!(
        database
            .try_acquire_workspace_lease(workspace_id, "replica-b", 131, Duration::from_secs(30))
            .await
            .unwrap()
    );
    database
        .release_workspace_lease(workspace_id, "replica-b")
        .await
        .unwrap();
    assert!(
        database
            .try_acquire_workspace_lease(workspace_id, "replica-c", 132, Duration::from_secs(30))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn deferred_job_is_not_reclaimed_before_new_availability() {
    let database = database().await;
    let job_id = database
        .enqueue_job(
            NewJob {
                kind: "reconcile_workspace".to_owned(),
                workspace_id: Some(Uuid::now_v7()),
                payload: Value::Null,
                available_at: 10,
            },
            10,
        )
        .await
        .unwrap();
    database
        .claim_job("replica-a", 10, Duration::from_secs(30))
        .await
        .unwrap();
    database
        .defer_job(job_id, "replica-a", 50, 11)
        .await
        .unwrap();
    assert!(
        database
            .claim_job("replica-b", 49, Duration::from_secs(30))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        database
            .claim_job("replica-b", 50, Duration::from_secs(30))
            .await
            .unwrap()
            .unwrap()
            .id,
        job_id
    );
}
