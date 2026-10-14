use std::{sync::Arc, time::Duration};

use thiserror::Error;
use tokio::sync::watch;

use crate::storage::{ClaimedJob, Database, StorageError};

mod kubernetes;
mod webhook;

const MAX_JOB_ATTEMPTS: i64 = 10;
const PENDING_JOB_DELAY_SECONDS: i64 = 5;

pub use kubernetes::WorkspaceReconcileHandler;
pub use webhook::{ControlPlaneJobHandler, WebhookDeliveryHandler};

pub trait JobHandler: Send + Sync + 'static {
    fn handle(
        &self,
        job: &ClaimedJob,
    ) -> impl std::future::Future<Output = Result<(), JobHandlerError>> + Send;

    fn on_terminal_failure(
        &self,
        _job: &ClaimedJob,
        _now: i64,
    ) -> impl std::future::Future<Output = Result<(), StorageError>> + Send {
        async { Ok(()) }
    }
}

pub struct JobWorker<H> {
    database: Database,
    handler: Arc<H>,
    lease_owner: String,
    poll_interval: Duration,
    job_lease_duration: Duration,
    workspace_lease_duration: Duration,
}

impl<H: JobHandler> JobWorker<H> {
    pub fn new(database: Database, handler: Arc<H>, lease_owner: String) -> Self {
        Self {
            database,
            handler,
            lease_owner,
            poll_interval: Duration::from_secs(1),
            job_lease_duration: Duration::from_secs(60),
            workspace_lease_duration: Duration::from_secs(60),
        }
    }

    pub async fn run_once(&self, now: i64) -> Result<bool, JobWorkerError> {
        let Some(job) = self
            .database
            .claim_job(&self.lease_owner, now, self.job_lease_duration)
            .await?
        else {
            return Ok(false);
        };
        if let Some(workspace_id) = job.workspace_id
            && !self
                .database
                .try_acquire_workspace_lease(
                    workspace_id,
                    &self.lease_owner,
                    now,
                    self.workspace_lease_duration,
                )
                .await?
        {
            self.database
                .defer_job_without_attempt(
                    job.id,
                    &self.lease_owner,
                    now.saturating_add(PENDING_JOB_DELAY_SECONDS),
                    now,
                )
                .await?;
            return Ok(true);
        }

        let result = self.handle_with_lease_heartbeat(&job).await?;
        let persistence_result = self.persist_job_result(&job, result, now).await;
        if let Some(workspace_id) = job.workspace_id {
            self.database
                .release_workspace_lease(workspace_id, &self.lease_owner)
                .await?;
        }
        persistence_result?;
        Ok(true)
    }

    async fn persist_job_result(
        &self,
        job: &ClaimedJob,
        result: Result<(), JobHandlerError>,
        now: i64,
    ) -> Result<(), StorageError> {
        match result {
            Ok(()) => {
                self.database
                    .complete_job(job.id, &self.lease_owner, now)
                    .await
            }
            Err(JobHandlerError::PendingWithCheckpoint { message, payload }) => {
                tracing::debug!(job_id = %job.id, error = %message, "job is waiting after applying resources");
                self.database
                    .save_job_checkpoint(job.id, &self.lease_owner, &payload, now)
                    .await?;
                self.database
                    .defer_job_without_attempt(
                        job.id,
                        &self.lease_owner,
                        now.saturating_add(1),
                        now,
                    )
                    .await
            }
            Err(JobHandlerError::Pending(error)) => {
                tracing::debug!(
                    job_id = %job.id,
                    attempts = job.attempts,
                    error = %error,
                    "job execution is waiting for external readiness"
                );
                self.database
                    .defer_job_without_attempt(
                        job.id,
                        &self.lease_owner,
                        now.saturating_add(PENDING_JOB_DELAY_SECONDS),
                        now,
                    )
                    .await
            }
            Err(error) => self.persist_failed_job(job, error, now).await,
        }
    }

    async fn persist_failed_job(
        &self,
        job: &ClaimedJob,
        error: JobHandlerError,
        now: i64,
    ) -> Result<(), StorageError> {
        tracing::warn!(job_id = %job.id, attempts = job.attempts, error = %error, "job execution failed");
        if job.attempts >= MAX_JOB_ATTEMPTS {
            tracing::error!(job_id = %job.id, attempts = job.attempts, "job reached the retry limit");
            self.handler.on_terminal_failure(job, now).await?;
            self.database.fail_job(job.id, &self.lease_owner, now).await
        } else {
            let delay = retry_delay(job.attempts);
            self.database
                .defer_job(job.id, &self.lease_owner, now.saturating_add(delay), now)
                .await
        }
    }

    async fn handle_with_lease_heartbeat(
        &self,
        job: &ClaimedJob,
    ) -> Result<Result<(), JobHandlerError>, JobWorkerError> {
        let handler = self.handler.handle(job);
        tokio::pin!(handler);
        let mut heartbeat = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_secs(20),
            Duration::from_secs(20),
        );
        loop {
            tokio::select! {
                result = &mut handler => return Ok(result),
                _ = heartbeat.tick() => {
                    let now = unix_timestamp()?;
                    self.database
                        .renew_job_lease(job.id, &self.lease_owner, now, self.job_lease_duration)
                        .await?;
                    if let Some(workspace_id) = job.workspace_id
                        && !self.database
                            .try_acquire_workspace_lease(
                                workspace_id,
                                &self.lease_owner,
                                now,
                                self.workspace_lease_duration,
                            )
                            .await?
                    {
                        return Err(JobWorkerError::WorkspaceLeaseLost(workspace_id));
                    }
                }
            }
        }
    }

    /// The installation setting changes admission live. Idle slots only wait on watch channels;
    /// in-flight work drains when concurrency decreases or the process shuts down.
    pub async fn run_pool_until_shutdown(
        &self,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<(), JobWorkerError> {
        let initial = self.database.job_worker_concurrency().await?;
        let (admission, receiver) = watch::channel(usize::from(initial));
        let (stop, stopped) = watch::channel(false);
        let mut tasks = tokio::task::JoinSet::new();
        for slot in 0..usize::from(crate::storage::MAX_JOB_WORKER_CONCURRENCY) {
            let worker = Self {
                database: self.database.clone(),
                handler: self.handler.clone(),
                lease_owner: format!("{}:worker-{slot}", self.lease_owner),
                poll_interval: self.poll_interval,
                job_lease_duration: self.job_lease_duration,
                workspace_lease_duration: self.workspace_lease_duration,
            };
            let concurrency = receiver.clone();
            let shutdown = shutdown.clone();
            let stop = stopped.clone();
            tasks
                .spawn(async move { worker.run_admitted(slot, concurrency, shutdown, stop).await });
        }
        let mut refresh = tokio::time::interval(self.poll_interval);
        refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let result = loop {
            if *shutdown.borrow() {
                break Ok(());
            }
            tokio::select! {
                _ = refresh.tick() => {
                    match self.database.job_worker_concurrency().await {
                        Ok(value) => { admission.send_if_modified(|current| {
                            let changed = *current != usize::from(value);
                            *current = usize::from(value);
                            changed
                        }); }
                        Err(error) => break Err(JobWorkerError::Storage(error)),
                    }
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { break Ok(()); }
                }
                Some(completed) = tasks.join_next() => {
                    break completed.map_err(JobWorkerError::Task).and_then(|result| result);
                }
            }
        };
        let _ = stop.send(true);
        while let Some(completed) = tasks.join_next().await {
            completed.map_err(JobWorkerError::Task)??;
        }
        result
    }

    async fn run_admitted(
        &self,
        slot: usize,
        mut concurrency: watch::Receiver<usize>,
        mut shutdown: watch::Receiver<bool>,
        mut stop: watch::Receiver<bool>,
    ) -> Result<(), JobWorkerError> {
        loop {
            if *shutdown.borrow() || *stop.borrow() {
                return Ok(());
            }
            let admitted = slot < *concurrency.borrow();
            if admitted && self.run_once(unix_timestamp()?).await? {
                continue;
            }
            let active = slot < *concurrency.borrow();
            tokio::select! {
                () = tokio::time::sleep(self.poll_interval), if active => {},
                changed = concurrency.changed() => {
                    if changed.is_err() { return Ok(()); }
                },
                changed = stop.changed() => {
                    if changed.is_err() || *stop.borrow() { return Ok(()); }
                },
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { return Ok(()); }
                }
            }
        }
    }

    pub async fn run_until_shutdown(
        &self,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<(), JobWorkerError> {
        loop {
            if *shutdown.borrow() {
                return Ok(());
            }
            let now = unix_timestamp()?;
            if self.run_once(now).await? {
                continue;
            }
            tokio::select! {
                () = tokio::time::sleep(self.poll_interval) => {},
                result = shutdown.changed() => {
                    if result.is_err() || *shutdown.borrow() {
                        return Ok(());
                    }
                }
            }
        }
    }
}

fn retry_delay(attempts: i64) -> i64 {
    let exponent = u32::try_from(attempts.clamp(0, 8)).unwrap_or(8);
    2_i64.pow(exponent).min(300)
}

fn unix_timestamp() -> Result<i64, JobWorkerError> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| JobWorkerError::Clock)?
        .as_secs();
    i64::try_from(seconds).map_err(|_| JobWorkerError::Clock)
}

#[derive(Debug, Error)]
pub enum JobHandlerError {
    #[error("{0}")]
    Failed(String),
    #[error("{0}")]
    Pending(String),
    #[error("{message}")]
    PendingWithCheckpoint {
        message: String,
        payload: serde_json::Value,
    },
}

#[derive(Debug, Error)]
pub enum JobWorkerError {
    #[error(transparent)]
    Task(#[from] tokio::task::JoinError),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("system clock is invalid")]
    Clock,
    #[error("workspace lease for {0} was lost during job execution")]
    WorkspaceLeaseLost(uuid::Uuid),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_delay_is_bounded_exponential() {
        assert_eq!(retry_delay(1), 2);
        assert_eq!(retry_delay(3), 8);
        assert_eq!(retry_delay(100), 256);
    }
}
