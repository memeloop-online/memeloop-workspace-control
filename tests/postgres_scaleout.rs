use std::{collections::HashSet, sync::Arc, time::Duration};

use memeloop_workspace_control::{
    events::NewEvent,
    storage::{Database, NewJob, StorageError},
};
use tokio::sync::Barrier;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn postgres_replicas_distribute_jobs_serialize_workspaces_and_notify_events() {
    let Ok(database_url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        eprintln!("skipping PostgreSQL integration test: MWC_TEST_POSTGRES_URL is not set");
        return;
    };
    let schema = format!("mwc_scaleout_{}", Uuid::now_v7().simple());
    let administration = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .unwrap();
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&administration)
        .await
        .unwrap();
    let mut scoped_url = url::Url::parse(&database_url).unwrap();
    scoped_url
        .query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema}"));
    let installation_id = "pg-scale-ci"
        .parse::<memeloop_workspace_control::config::InstallationId>()
        .unwrap();
    let replica_a = Database::connect(scoped_url.as_str(), installation_id.clone())
        .await
        .unwrap();
    let replica_b = Database::connect(scoped_url.as_str(), installation_id)
        .await
        .unwrap();

    let (migration_a, migration_b) = tokio::join!(replica_a.migrate(), replica_b.migrate());
    migration_a.unwrap();
    migration_b.unwrap();

    for sequence in 0..40 {
        replica_a
            .enqueue_job(
                NewJob {
                    kind: "scaleout-test".to_owned(),
                    workspace_id: None,
                    payload: serde_json::json!({"sequence": sequence}),
                    available_at: 100,
                },
                100,
            )
            .await
            .unwrap();
    }

    let barrier = Arc::new(Barrier::new(4));
    let mut workers = Vec::new();
    for index in 0..4 {
        let database = replica_a.clone();
        let barrier = barrier.clone();
        workers.push(tokio::spawn(async move {
            let owner = format!("replica-{index}");
            let mut claimed = Vec::new();
            barrier.wait().await;
            while let Some(job) = database
                .claim_job(&owner, 100, Duration::from_secs(30))
                .await
                .unwrap()
            {
                claimed.push(job.id);
                tokio::time::sleep(Duration::from_millis(2)).await;
                database.complete_job(job.id, &owner, 101).await.unwrap();
            }
            claimed
        }));
    }

    let mut all_claimed = HashSet::new();
    let mut active_replicas = 0;
    for worker in workers {
        let claimed = worker.await.unwrap();
        if !claimed.is_empty() {
            active_replicas += 1;
        }
        for job_id in claimed {
            assert!(
                all_claimed.insert(job_id),
                "job was executed more than once"
            );
        }
    }
    assert_eq!(all_claimed.len(), 40);
    assert_eq!(
        active_replicas, 4,
        "work was not distributed to every replica"
    );

    let expiring_job = replica_a
        .enqueue_job(
            NewJob {
                kind: "lease-recovery-test".to_owned(),
                workspace_id: None,
                payload: serde_json::Value::Null,
                available_at: 200,
            },
            200,
        )
        .await
        .unwrap();
    assert_eq!(
        replica_a
            .claim_job("expired-owner", 200, Duration::from_secs(2))
            .await
            .unwrap()
            .unwrap()
            .id,
        expiring_job
    );
    assert!(
        replica_b
            .claim_job("recovery-owner", 202, Duration::from_secs(2))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        replica_b
            .claim_job("recovery-owner", 203, Duration::from_secs(2))
            .await
            .unwrap()
            .unwrap()
            .id,
        expiring_job
    );

    let workspace_id = Uuid::now_v7();
    let mut lease_attempts = Vec::new();
    for index in 0..4 {
        let database = replica_a.clone();
        lease_attempts.push(tokio::spawn(async move {
            database
                .try_acquire_workspace_lease(
                    workspace_id,
                    &format!("workspace-replica-{index}"),
                    300,
                    Duration::from_secs(30),
                )
                .await
                .unwrap()
        }));
    }
    let mut acquired = 0;
    for attempt in lease_attempts {
        acquired += usize::from(attempt.await.unwrap());
    }
    assert_eq!(
        acquired, 1,
        "workspace lease admitted more than one replica"
    );

    let mut notifier = replica_b.event_notifier().await.unwrap();
    let notification =
        tokio::spawn(
            async move { tokio::time::timeout(Duration::from_secs(3), notifier.wait()).await },
        );
    let organization_id = Uuid::now_v7();
    replica_a
        .append_event(
            NewEvent {
                organization_id,
                workspace_id: Some(workspace_id),
                kind: "scaleout.notified".to_owned(),
                payload: serde_json::json!({"replica": "a"}),
            },
            400,
        )
        .await
        .unwrap();
    notification.await.unwrap().unwrap().unwrap();
    assert_eq!(
        replica_b
            .list_events(organization_id, None, 10)
            .await
            .unwrap()
            .len(),
        1
    );

    let wrong_installation = Database::connect(scoped_url.as_str(), "pg-other".parse().unwrap())
        .await
        .unwrap();
    assert!(matches!(
        wrong_installation.migrate().await,
        Err(StorageError::InstallationMismatch { stored, .. }) if stored == "pg-scale-ci"
    ));

    drop(wrong_installation);
    drop(replica_b);
    drop(replica_a);
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&administration)
        .await
        .unwrap();
    administration.close().await;
}
