use super::*;

use crate::storage::NewJob;
use futures::{FutureExt, poll};
use serde_json::json;
use tokio::sync::{Semaphore, mpsc};

struct Handler {
    started: mpsc::UnboundedSender<Uuid>,
    release: Arc<Semaphore>,
}

impl JobHandler for Handler {
    async fn handle(&self, job: &ClaimedJob) -> Result<(), JobHandlerError> {
        self.started.send(job.id).unwrap();
        self.release.acquire().await.unwrap().forget();
        Ok(())
    }
}

async fn database() -> Database {
    let database = Database::connect("sqlite::memory:", "worker-wakeup-test".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
}

async fn enqueue(database: &Database, kind: &str, available_at: i64) -> Uuid {
    database
        .enqueue_job(
            NewJob {
                kind: kind.to_owned(),
                workspace_id: None,
                payload: json!({}),
                available_at,
            },
            0,
        )
        .await
        .unwrap()
}

fn worker(
    database: &Database,
    permits: usize,
) -> (JobWorker<Handler>, mpsc::UnboundedReceiver<Uuid>) {
    let (started, receiver) = mpsc::unbounded_channel();
    let handler = Arc::new(Handler {
        started,
        release: Arc::new(Semaphore::new(permits)),
    });
    let mut worker = JobWorker::new(database.clone(), handler, "wakeup-worker".into())
        .with_job_wakeup(Arc::new(Notify::new()));
    worker.poll_interval = Duration::from_secs(60);
    (worker, receiver)
}

// The first real claim returns no row while atomically making the other jobs due.
// This exposes the empty-claim/wait boundary without a product-side test hook.
async fn empty_claim_handoff(database: &Database) {
    enqueue(database, "handoff", 0).await;
    let Database::Sqlite { pool, .. } = database else {
        unreachable!()
    };
    sqlx::query(
        "CREATE TRIGGER empty_claim_handoff BEFORE UPDATE OF status ON jobs \
         WHEN OLD.kind = 'handoff' BEGIN \
         DELETE FROM jobs WHERE id = OLD.id; \
         UPDATE jobs SET available_at = 0 WHERE kind = 'test' \
         AND NOT EXISTS (SELECT 1 FROM jobs WHERE kind = 'handoff'); \
         SELECT RAISE(IGNORE); END",
    )
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn wake_during_empty_claim_is_retained_by_both_worker_runners() {
    for pooled in [false, true] {
        let database = database().await;
        let expected = enqueue(&database, "test", i64::MAX).await;
        empty_claim_handoff(&database).await;
        let (worker, mut started) = worker(&database, 1);
        let (shutdown, receiver) = watch::channel(false);
        let (_admission, admitted) = watch::channel(1);
        let (_stop, stopped) = watch::channel(false);
        let mut run = if pooled {
            worker.run_admitted(0, admitted, receiver, stopped).boxed()
        } else {
            worker.run_until_shutdown(receiver).boxed()
        };
        let Database::Sqlite { pool, .. } = &database else {
            unreachable!()
        };
        let connection = pool.acquire().await.unwrap();
        // The actual worker has registered its notification and is awaiting claim SQL.
        assert!(poll!(run.as_mut()).is_pending());
        worker.job_wakeup.notify_waiters();
        drop(connection);
        tokio::time::timeout(Duration::from_secs(5), async {
            tokio::select! {
                id = started.recv() => assert_eq!(id, Some(expected)),
                result = run.as_mut() => panic!("worker exited before executing: {result:?}"),
            }
        })
        .await
        .expect("notification must beat the 60s fallback after an empty claim");
        shutdown.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(database.job_counts().await.unwrap().completed, 1);
    }
}

#[tokio::test]
async fn notified_pool_fills_admitted_slots_and_drains_without_admitting_more() {
    let database = database().await;
    database.set_job_worker_concurrency(3, 0).await.unwrap();
    for _ in 0..4 {
        enqueue(&database, "test", i64::MAX).await;
    }
    enqueue(&database, "handoff", 0).await;
    empty_claim_handoff(&database).await;
    let (worker, mut started) = worker(&database, 0);
    let wakeup = worker.job_wakeup.clone();
    let release = worker.handler.release.clone();
    let (shutdown, receiver) = watch::channel(false);
    let task = tokio::spawn(async move { worker.run_pool_until_shutdown(receiver).await });
    // Two slots return empty claims; the third claims a due job and stays in flight.
    tokio::time::timeout(Duration::from_secs(5), started.recv())
        .await
        .unwrap()
        .unwrap();
    wakeup.notify_waiters();
    for _ in 0..2 {
        tokio::time::timeout(Duration::from_secs(5), started.recv())
            .await
            .expect("both idle admitted slots must wake before their 60s poll")
            .unwrap();
    }
    let counts = database.job_counts().await.unwrap();
    assert_eq!((counts.running, counts.pending), (3, 1));
    shutdown.send(true).unwrap();
    release.add_permits(3);
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let counts = database.job_counts().await.unwrap();
    assert_eq!(
        (counts.completed, counts.pending, counts.running),
        (3, 1, 0)
    );
    assert!(started.try_recv().is_err());
}

#[tokio::test]
async fn job_notification_cannot_bypass_live_admission() {
    let database = database().await;
    let expected = enqueue(&database, "test", 0).await;
    let (worker, mut started) = worker(&database, 1);
    let (admission, admitted) = watch::channel(0);
    let (_stop, stopped) = watch::channel(false);
    let (shutdown, receiver) = watch::channel(false);
    let run = worker.run_admitted(0, admitted, receiver, stopped);
    tokio::pin!(run);
    assert!(poll!(run.as_mut()).is_pending());
    worker.job_wakeup.notify_waiters();
    assert!(poll!(run.as_mut()).is_pending());
    assert_eq!(database.job_counts().await.unwrap().pending, 1);
    admission.send(1).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            id = started.recv() => assert_eq!(id, Some(expected)),
            result = run.as_mut() => panic!("worker exited before executing: {result:?}"),
        }
    })
    .await
    .unwrap();
    shutdown.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn worker_fallback_polls_without_notification() {
    let database = database().await;
    let expected = enqueue(&database, "test", i64::MAX).await;
    empty_claim_handoff(&database).await;
    let (mut worker, mut started) = worker(&database, 1);
    worker.poll_interval = Duration::from_millis(20);
    let (shutdown, receiver) = watch::channel(false);
    let run = worker.run_until_shutdown(receiver);
    tokio::pin!(run);
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            id = started.recv() => assert_eq!(id, Some(expected)),
            result = run.as_mut() => panic!("worker exited before executing: {result:?}"),
        }
    })
    .await
    .unwrap();
    shutdown.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn shutdown_interrupts_idle_worker_with_long_fallback() {
    let database = database().await;
    let (worker, _started) = worker(&database, 0);
    let (shutdown, receiver) = watch::channel(false);
    let run = worker.run_until_shutdown(receiver);
    tokio::pin!(run);
    assert!(poll!(run.as_mut()).is_pending());
    // Ping queues behind the real claim; keep driving the worker until the claim
    // has released its connection and the empty worker is waiting again.
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            result = database.ping() => result.unwrap(),
            result = run.as_mut() => panic!("idle worker exited early: {result:?}"),
        }
    })
    .await
    .unwrap();
    shutdown.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .expect("shutdown must not wait for the 60s fallback")
        .unwrap();
}
