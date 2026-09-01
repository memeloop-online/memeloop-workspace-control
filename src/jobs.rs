use std::{sync::Arc, time::Duration};

use thiserror::Error;
use tokio::sync::watch;

use crate::storage::{ClaimedJob, Database, StorageError};

mod kubernetes;
mod webhook;

const MAX_JOB_ATTEMPTS: i64 = 10;

pub use kubernetes::WorkspaceReconcileHandler;
pub use webhook::{ControlPlaneJobHandler, WebhookDeliveryHandler};

#[allow(async_fn_in_trait)]
pub trait JobHandler: Send + Sync + 'static {
    async fn handle(&self, job: &ClaimedJob) -> Result<(), JobHandlerError>;
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
                .defer_job(job.id, &self.lease_owner, now.saturating_add(5), now)
                .await?;
            return Ok(true);
        }

        let result = self.handle_with_lease_heartbeat(&job).await?;
        let persistence_result = match result {
            Ok(()) => {
                self.database
                    .complete_job(job.id, &self.lease_owner, now)
                    .await
            }
            Err(error) => {
                tracing::warn!(job_id = %job.id, attempts = job.attempts, error = %error, "job execution failed");
                if job.attempts >= MAX_JOB_ATTEMPTS {
                    tracing::error!(job_id = %job.id, attempts = job.attempts, "job reached the retry limit");
                    self.database.fail_job(job.id, &self.lease_owner, now).await
                } else {
                    let delay = retry_delay(job.attempts);
                    self.database
                        .defer_job(job.id, &self.lease_owner, now.saturating_add(delay), now)
                        .await
                }
            }
        };
        if let Some(workspace_id) = job.workspace_id {
            self.database
                .release_workspace_lease(workspace_id, &self.lease_owner)
                .await?;
        }
        persistence_result?;
        Ok(true)
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
#[error("{0}")]
pub struct JobHandlerError(pub String);

#[derive(Debug, Error)]
pub enum JobWorkerError {
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
