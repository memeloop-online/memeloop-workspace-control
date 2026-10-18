use super::{Database, StorageError};

pub const DEFAULT_JOB_WORKER_CONCURRENCY: u16 = 32;
pub const MAX_JOB_WORKER_CONCURRENCY: u16 = 256;
pub const DEFAULT_CPU_OVERCOMMIT_RATIO: u16 = 10;
pub const MAX_CPU_OVERCOMMIT_RATIO: u16 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeSettings {
    pub job_worker_concurrency: u16,
    pub cpu_overcommit_ratio: u16,
}

impl Default for RuntimeSettings {
    fn default() -> Self {
        Self {
            job_worker_concurrency: DEFAULT_JOB_WORKER_CONCURRENCY,
            cpu_overcommit_ratio: DEFAULT_CPU_OVERCOMMIT_RATIO,
        }
    }
}

impl RuntimeSettings {
    fn from_row((concurrency, ratio): (i64, i64)) -> Result<Self, StorageError> {
        let job_worker_concurrency = u16::try_from(concurrency)
            .ok()
            .filter(|value| (1..=MAX_JOB_WORKER_CONCURRENCY).contains(value))
            .ok_or(StorageError::InvalidJobWorkerConcurrency)?;
        let cpu_overcommit_ratio = u16::try_from(ratio)
            .ok()
            .filter(|value| (1..=MAX_CPU_OVERCOMMIT_RATIO).contains(value))
            .ok_or(StorageError::InvalidCpuOvercommitRatio)?;
        Ok(Self {
            job_worker_concurrency,
            cpu_overcommit_ratio,
        })
    }
}

impl Database {
    pub async fn runtime_settings(&self) -> Result<RuntimeSettings, StorageError> {
        let value: Option<(i64, i64)> = match self {
            Self::Sqlite { pool, installation_id } => sqlx::query_as(
                "SELECT job_worker_concurrency, cpu_overcommit_ratio FROM installation_runtime_settings WHERE installation_id = ?1",
            ).bind(installation_id.as_str()).fetch_optional(pool).await?,
            Self::Postgres { pool, installation_id } => sqlx::query_as(
                "SELECT job_worker_concurrency, cpu_overcommit_ratio FROM installation_runtime_settings WHERE installation_id = $1",
            ).bind(installation_id.as_str()).fetch_optional(pool).await?,
        };
        value.map_or_else(|| Ok(RuntimeSettings::default()), RuntimeSettings::from_row)
    }

    pub async fn job_worker_concurrency(&self) -> Result<u16, StorageError> {
        Ok(self.runtime_settings().await?.job_worker_concurrency)
    }

    pub async fn cpu_overcommit_ratio(&self) -> Result<u16, StorageError> {
        Ok(self.runtime_settings().await?.cpu_overcommit_ratio)
    }

    /// Patch both values in one atomic statement. Never read/modify/write a whole
    /// settings row: concurrent patches to different fields must preserve each other.
    /// CPU settings are consumed at workspace creation; this does not enqueue jobs.
    pub async fn patch_runtime_settings(
        &self,
        concurrency: Option<u16>,
        cpu_ratio: Option<u16>,
        now: i64,
    ) -> Result<RuntimeSettings, StorageError> {
        if concurrency.is_some_and(|value| !(1..=MAX_JOB_WORKER_CONCURRENCY).contains(&value)) {
            return Err(StorageError::InvalidJobWorkerConcurrency);
        }
        if cpu_ratio.is_some_and(|value| !(1..=MAX_CPU_OVERCOMMIT_RATIO).contains(&value)) {
            return Err(StorageError::InvalidCpuOvercommitRatio);
        }
        if concurrency.is_none() && cpu_ratio.is_none() {
            return self.runtime_settings().await;
        }
        let row = match self {
            Self::Sqlite { pool, installation_id } => sqlx::query_as(
                "INSERT INTO installation_runtime_settings (installation_id, job_worker_concurrency, cpu_overcommit_ratio, updated_at) VALUES (?1, COALESCE(?2, ?5), COALESCE(?3, ?6), ?4) ON CONFLICT (installation_id) DO UPDATE SET job_worker_concurrency = COALESCE(?2, installation_runtime_settings.job_worker_concurrency), cpu_overcommit_ratio = COALESCE(?3, installation_runtime_settings.cpu_overcommit_ratio), updated_at = excluded.updated_at RETURNING job_worker_concurrency, cpu_overcommit_ratio",
            ).bind(installation_id.as_str()).bind(concurrency.map(i64::from)).bind(cpu_ratio.map(i64::from)).bind(now)
                .bind(i64::from(DEFAULT_JOB_WORKER_CONCURRENCY)).bind(i64::from(DEFAULT_CPU_OVERCOMMIT_RATIO)).fetch_one(pool).await?,
            Self::Postgres { pool, installation_id } => sqlx::query_as(
                "INSERT INTO installation_runtime_settings (installation_id, job_worker_concurrency, cpu_overcommit_ratio, updated_at) VALUES ($1, COALESCE($2, $5), COALESCE($3, $6), $4) ON CONFLICT (installation_id) DO UPDATE SET job_worker_concurrency = COALESCE($2, installation_runtime_settings.job_worker_concurrency), cpu_overcommit_ratio = COALESCE($3, installation_runtime_settings.cpu_overcommit_ratio), updated_at = excluded.updated_at RETURNING job_worker_concurrency, cpu_overcommit_ratio",
            ).bind(installation_id.as_str()).bind(concurrency.map(i64::from)).bind(cpu_ratio.map(i64::from)).bind(now)
                .bind(i64::from(DEFAULT_JOB_WORKER_CONCURRENCY)).bind(i64::from(DEFAULT_CPU_OVERCOMMIT_RATIO)).fetch_one(pool).await?,
        };
        RuntimeSettings::from_row(row)
    }

    pub async fn set_job_worker_concurrency(
        &self,
        value: u16,
        now: i64,
    ) -> Result<(), StorageError> {
        self.patch_runtime_settings(Some(value), None, now).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
