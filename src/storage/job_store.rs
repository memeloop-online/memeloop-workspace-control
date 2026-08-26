use std::time::Duration;

use uuid::Uuid;

use super::{
    ClaimedJob, Database, NewJob, StorageError,
    job_types::{decode_postgres_job, decode_sqlite_job},
};

impl Database {
    pub async fn enqueue_job(&self, new_job: NewJob, now: i64) -> Result<Uuid, StorageError> {
        let id = Uuid::now_v7();
        let payload_json = serde_json::to_string(&new_job.payload)?;
        let workspace_id = new_job.workspace_id.map(|value| value.to_string());
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query(
                    "INSERT INTO jobs \
                    (id, installation_id, kind, workspace_id, payload_json, status, available_at, \
                    lease_owner, lease_expires_at, attempts, created_at, updated_at) \
                    VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, NULL, NULL, 0, ?7, ?7)",
                )
                .bind(id.to_string())
                .bind(installation_id.as_str())
                .bind(new_job.kind)
                .bind(workspace_id)
                .bind(payload_json)
                .bind(new_job.available_at)
                .bind(now)
                .execute(pool)
                .await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query(
                    "INSERT INTO jobs \
                    (id, installation_id, kind, workspace_id, payload_json, status, available_at, \
                    lease_owner, lease_expires_at, attempts, created_at, updated_at) \
                    VALUES ($1, $2, $3, $4, $5, 'pending', $6, NULL, NULL, 0, $7, $7)",
                )
                .bind(id.to_string())
                .bind(installation_id.as_str())
                .bind(new_job.kind)
                .bind(workspace_id)
                .bind(payload_json)
                .bind(new_job.available_at)
                .bind(now)
                .execute(pool)
                .await?;
            }
        }
        Ok(id)
    }

    pub async fn claim_job(
        &self,
        lease_owner: &str,
        now: i64,
        lease_duration: Duration,
    ) -> Result<Option<ClaimedJob>, StorageError> {
        if lease_owner.is_empty() {
            return Err(StorageError::EmptyLeaseOwner);
        }
        let lease_seconds = i64::try_from(lease_duration.as_secs())
            .map_err(|_| StorageError::LeaseDurationOverflow)?;
        let lease_expires_at = now
            .checked_add(lease_seconds)
            .ok_or(StorageError::LeaseDurationOverflow)?;

        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let row = sqlx::query(
                    "UPDATE jobs SET status = 'running', lease_owner = ?1, \
                    lease_expires_at = ?2, attempts = attempts + 1, updated_at = ?3 \
                    WHERE id = (SELECT id FROM jobs WHERE installation_id = ?4 AND \
                    ((status = 'pending' AND available_at <= ?3) OR \
                    (status = 'running' AND lease_expires_at < ?3)) \
                    ORDER BY available_at, id LIMIT 1) \
                    RETURNING id, kind, workspace_id, payload_json, attempts, lease_expires_at",
                )
                .bind(lease_owner)
                .bind(lease_expires_at)
                .bind(now)
                .bind(installation_id.as_str())
                .fetch_optional(pool)
                .await?;
                row.map(decode_sqlite_job).transpose()
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let row = sqlx::query(
                    "WITH candidate AS (SELECT id FROM jobs WHERE installation_id = $1 AND \
                    ((status = 'pending' AND available_at <= $2) OR \
                    (status = 'running' AND lease_expires_at < $2)) \
                    ORDER BY available_at, id FOR UPDATE SKIP LOCKED LIMIT 1) \
                    UPDATE jobs SET status = 'running', lease_owner = $3, \
                    lease_expires_at = $4, attempts = jobs.attempts + 1, updated_at = $2 \
                    FROM candidate WHERE jobs.id = candidate.id \
                    RETURNING jobs.id, jobs.kind, jobs.workspace_id, jobs.payload_json, \
                    jobs.attempts, jobs.lease_expires_at",
                )
                .bind(installation_id.as_str())
                .bind(now)
                .bind(lease_owner)
                .bind(lease_expires_at)
                .fetch_optional(pool)
                .await?;
                row.map(decode_postgres_job).transpose()
            }
        }
    }

    pub async fn complete_job(
        &self,
        job_id: Uuid,
        lease_owner: &str,
        now: i64,
    ) -> Result<(), StorageError> {
        let rows_affected = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(
                "UPDATE jobs SET status = 'completed', lease_owner = NULL, \
                lease_expires_at = NULL, updated_at = ?1 WHERE id = ?2 AND \
                installation_id = ?3 AND status = 'running' AND lease_owner = ?4",
            )
            .bind(now)
            .bind(job_id.to_string())
            .bind(installation_id.as_str())
            .bind(lease_owner)
            .execute(pool)
            .await?
            .rows_affected(),
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(
                "UPDATE jobs SET status = 'completed', lease_owner = NULL, \
                lease_expires_at = NULL, updated_at = $1 WHERE id = $2 AND \
                installation_id = $3 AND status = 'running' AND lease_owner = $4",
            )
            .bind(now)
            .bind(job_id.to_string())
            .bind(installation_id.as_str())
            .bind(lease_owner)
            .execute(pool)
            .await?
            .rows_affected(),
        };
        if rows_affected != 1 {
            return Err(StorageError::LeaseNotOwned(job_id));
        }
        Ok(())
    }

    pub async fn renew_job_lease(
        &self,
        job_id: Uuid,
        lease_owner: &str,
        now: i64,
        lease_duration: Duration,
    ) -> Result<(), StorageError> {
        let lease_expires_at = now
            .checked_add(
                i64::try_from(lease_duration.as_secs())
                    .map_err(|_| StorageError::LeaseDurationOverflow)?,
            )
            .ok_or(StorageError::LeaseDurationOverflow)?;
        let affected = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query("UPDATE jobs SET lease_expires_at = ?1, updated_at = ?2 WHERE id = ?3 AND installation_id = ?4 AND status = 'running' AND lease_owner = ?5")
                .bind(lease_expires_at).bind(now).bind(job_id.to_string())
                .bind(installation_id.as_str()).bind(lease_owner).execute(pool).await?.rows_affected(),
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query("UPDATE jobs SET lease_expires_at = $1, updated_at = $2 WHERE id = $3 AND installation_id = $4 AND status = 'running' AND lease_owner = $5")
                .bind(lease_expires_at).bind(now).bind(job_id.to_string())
                .bind(installation_id.as_str()).bind(lease_owner).execute(pool).await?.rows_affected(),
        };
        if affected != 1 {
            return Err(StorageError::LeaseNotOwned(job_id));
        }
        Ok(())
    }

    pub async fn defer_job(
        &self,
        job_id: Uuid,
        lease_owner: &str,
        available_at: i64,
        now: i64,
    ) -> Result<(), StorageError> {
        let affected = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(
                "UPDATE jobs SET status = 'pending', available_at = ?1, lease_owner = NULL, \
                lease_expires_at = NULL, updated_at = ?2 WHERE id = ?3 AND installation_id = ?4 \
                AND status = 'running' AND lease_owner = ?5",
            )
            .bind(available_at)
            .bind(now)
            .bind(job_id.to_string())
            .bind(installation_id.as_str())
            .bind(lease_owner)
            .execute(pool)
            .await?
            .rows_affected(),
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(
                "UPDATE jobs SET status = 'pending', available_at = $1, lease_owner = NULL, \
                lease_expires_at = NULL, updated_at = $2 WHERE id = $3 AND installation_id = $4 \
                AND status = 'running' AND lease_owner = $5",
            )
            .bind(available_at)
            .bind(now)
            .bind(job_id.to_string())
            .bind(installation_id.as_str())
            .bind(lease_owner)
            .execute(pool)
            .await?
            .rows_affected(),
        };
        if affected != 1 {
            return Err(StorageError::LeaseNotOwned(job_id));
        }
        Ok(())
    }
}
