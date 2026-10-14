use super::{Database, StorageError};

pub const DEFAULT_JOB_WORKER_CONCURRENCY: u16 = 32;
pub const MAX_JOB_WORKER_CONCURRENCY: u16 = 256;

impl Database {
    pub async fn job_worker_concurrency(&self) -> Result<u16, StorageError> {
        let value: Option<i64> = match self {
            Self::Sqlite { pool, installation_id } => sqlx::query_scalar(
                "SELECT job_worker_concurrency FROM installation_runtime_settings WHERE installation_id = ?1",
            ).bind(installation_id.as_str()).fetch_optional(pool).await?,
            Self::Postgres { pool, installation_id } => sqlx::query_scalar(
                "SELECT job_worker_concurrency FROM installation_runtime_settings WHERE installation_id = $1",
            ).bind(installation_id.as_str()).fetch_optional(pool).await?,
        };
        value.map_or(Ok(DEFAULT_JOB_WORKER_CONCURRENCY), |value| {
            u16::try_from(value)
                .ok()
                .filter(|value| (1..=MAX_JOB_WORKER_CONCURRENCY).contains(value))
                .ok_or(StorageError::InvalidJobWorkerConcurrency)
        })
    }

    pub async fn set_job_worker_concurrency(
        &self,
        value: u16,
        now: i64,
    ) -> Result<(), StorageError> {
        if !(1..=MAX_JOB_WORKER_CONCURRENCY).contains(&value) {
            return Err(StorageError::InvalidJobWorkerConcurrency);
        }
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO installation_runtime_settings (installation_id, job_worker_concurrency, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT (installation_id) DO UPDATE SET job_worker_concurrency = excluded.job_worker_concurrency, updated_at = excluded.updated_at")
                    .bind(installation_id.as_str()).bind(i64::from(value)).bind(now).execute(pool).await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO installation_runtime_settings (installation_id, job_worker_concurrency, updated_at) VALUES ($1, $2, $3) ON CONFLICT (installation_id) DO UPDATE SET job_worker_concurrency = excluded.job_worker_concurrency, updated_at = excluded.updated_at")
                    .bind(installation_id.as_str()).bind(i64::from(value)).bind(now).execute(pool).await?;
            }
        }
        Ok(())
    }
}
