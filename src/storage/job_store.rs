use std::{collections::HashSet, time::Duration};

use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use super::{
    ClaimedJob, Database, NewJob, ParkedWorkspaceJob, ParkedWorkspaceJobsPage, StorageError,
    job_types::{
        decode_postgres_job, decode_postgres_parked_job, decode_sqlite_job,
        decode_sqlite_parked_job,
    },
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

    pub async fn fail_job(
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
                "UPDATE jobs SET status = 'failed', lease_owner = NULL, \
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
                "UPDATE jobs SET status = 'failed', lease_owner = NULL, \
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
                lease_expires_at = NULL, payload_json = CASE \
                WHEN json_type(payload_json, '$.runtime_wait') IS NOT NULL \
                THEN json_remove(payload_json, '$.runtime_wait') ELSE payload_json END, \
                updated_at = ?2 WHERE id = ?3 AND installation_id = ?4 \
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
                lease_expires_at = NULL, payload_json = CASE \
                WHEN payload_json::jsonb ? 'runtime_wait' \
                THEN (payload_json::jsonb - 'runtime_wait')::text ELSE payload_json END, \
                updated_at = $2 WHERE id = $3 AND installation_id = $4 \
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

    /// Persist a new park identity and release the running lease in the same statement.
    pub async fn park_workspace_job(
        &self,
        job_id: Uuid,
        lease_owner: &str,
        payload: &Value,
        until: i64,
        now: i64,
    ) -> Result<Value, StorageError> {
        let mut stored: serde_json::Map<String, Value> = serde_json::from_value(payload.clone())?;
        stored.insert(
            "runtime_wait".to_owned(),
            serde_json::json!({"nonce": Uuid::now_v7(), "until": until}),
        );
        let payload_json = serde_json::to_string(&stored)?;
        let stored_json: Option<String> = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query_scalar(
                    "UPDATE jobs SET payload_json = ?1, status = 'pending', available_at = ?2, \
                attempts = CASE WHEN attempts > 0 THEN attempts - 1 ELSE 0 END, \
                lease_owner = NULL, lease_expires_at = NULL, updated_at = ?3 \
                WHERE id = ?4 AND installation_id = ?5 AND status = 'running' \
                AND lease_owner = ?6 RETURNING payload_json",
                )
                .bind(&payload_json)
                .bind(until)
                .bind(now)
                .bind(job_id.to_string())
                .bind(installation_id.as_str())
                .bind(lease_owner)
                .fetch_optional(pool)
                .await?
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query_scalar(
                    "UPDATE jobs SET payload_json = $1, status = 'pending', available_at = $2, \
                attempts = CASE WHEN attempts > 0 THEN attempts - 1 ELSE 0 END, \
                lease_owner = NULL, lease_expires_at = NULL, updated_at = $3 \
                WHERE id = $4 AND installation_id = $5 AND status = 'running' \
                AND lease_owner = $6 RETURNING payload_json",
                )
                .bind(&payload_json)
                .bind(until)
                .bind(now)
                .bind(job_id.to_string())
                .bind(installation_id.as_str())
                .bind(lease_owner)
                .fetch_optional(pool)
                .await?
            }
        };
        let stored_json = stored_json.ok_or(StorageError::LeaseNotOwned(job_id))?;
        Ok(serde_json::from_str(&stored_json)?)
    }

    /// Scan one bounded raw page. Rejected markers still advance `next`.
    pub async fn list_parked_workspace_jobs(
        &self,
        after: Option<Uuid>,
        limit: u32,
        now: i64,
    ) -> Result<ParkedWorkspaceJobsPage, StorageError> {
        let limit = i64::from(limit.clamp(1, 1_000));
        let after = after.map(|id| id.to_string());
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let rows = sqlx::query(
                    "SELECT j.id, j.workspace_id, j.payload_json, j.available_at, \
                    w.generation AS workspace_generation, w.state FROM jobs j \
                    JOIN workspaces w ON w.id = j.workspace_id \
                    AND w.installation_id = j.installation_id \
                    WHERE j.installation_id = ?1 AND j.kind = 'reconcile_workspace' \
                    AND j.status = 'pending' AND j.available_at > ?2 \
                    AND (?3 IS NULL OR j.id > ?3) \
                    AND w.state IN ('provisioning', 'starting', 'restarting', 'stopping') \
                    AND json_type(j.payload_json, '$.runtime_wait') IS NOT NULL \
                    AND json_extract(j.payload_json, '$.reconciled_generation') = w.generation \
                    ORDER BY j.id LIMIT ?4",
                )
                .bind(installation_id.as_str())
                .bind(now)
                .bind(&after)
                .bind(limit)
                .fetch_all(pool)
                .await?;
                let next = if rows.len() == limit as usize {
                    rows.last()
                        .map(|row| row.try_get::<String, _>("id"))
                        .transpose()?
                        .map(|id| Uuid::parse_str(&id))
                        .transpose()?
                } else {
                    None
                };
                let jobs = rows
                    .into_iter()
                    .filter_map(|row| decode_sqlite_parked_job(row).transpose())
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(ParkedWorkspaceJobsPage { jobs, next })
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let rows = sqlx::query(
                    "SELECT j.id, j.workspace_id, j.payload_json, j.available_at, \
                    w.generation AS workspace_generation, w.state FROM jobs j \
                    JOIN workspaces w ON w.id = j.workspace_id \
                    AND w.installation_id = j.installation_id \
                    WHERE j.installation_id = $1 AND j.kind = 'reconcile_workspace' \
                    AND j.status = 'pending' AND j.available_at > $2 \
                    AND ($3::text IS NULL OR j.id > $3) \
                    AND w.state IN ('provisioning', 'starting', 'restarting', 'stopping') \
                    AND j.payload_json::jsonb ? 'runtime_wait' \
                    AND j.payload_json::jsonb ->> 'reconciled_generation' = w.generation::text \
                    ORDER BY j.id LIMIT $4",
                )
                .bind(installation_id.as_str())
                .bind(now)
                .bind(&after)
                .bind(limit)
                .fetch_all(pool)
                .await?;
                let next = if rows.len() == limit as usize {
                    rows.last()
                        .map(|row| row.try_get::<String, _>("id"))
                        .transpose()?
                        .map(|id| Uuid::parse_str(&id))
                        .transpose()?
                } else {
                    None
                };
                let jobs = rows
                    .into_iter()
                    .filter_map(|row| decode_postgres_parked_job(row).transpose())
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(ParkedWorkspaceJobsPage { jobs, next })
            }
        }
    }

    /// Rebuild the persisted descriptor for a post-park hook without scanning other jobs.
    pub async fn get_parked_workspace_job(
        &self,
        job_id: Uuid,
        now: i64,
    ) -> Result<Option<ParkedWorkspaceJob>, StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let row = sqlx::query(
                    "SELECT j.id, j.workspace_id, j.payload_json, j.available_at, \
                    w.generation AS workspace_generation, w.state FROM jobs j \
                    JOIN workspaces w ON w.id = j.workspace_id \
                    AND w.installation_id = j.installation_id \
                    WHERE j.id = ?1 AND j.installation_id = ?2 \
                    AND j.kind = 'reconcile_workspace' AND j.status = 'pending' \
                    AND j.available_at > ?3 \
                    AND w.state IN ('provisioning', 'starting', 'restarting', 'stopping') \
                    AND json_type(j.payload_json, '$.runtime_wait') IS NOT NULL \
                    AND json_extract(j.payload_json, '$.reconciled_generation') = w.generation",
                )
                .bind(job_id.to_string())
                .bind(installation_id.as_str())
                .bind(now)
                .fetch_optional(pool)
                .await?;
                row.map(decode_sqlite_parked_job)
                    .transpose()
                    .map(Option::flatten)
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let row = sqlx::query(
                    "SELECT j.id, j.workspace_id, j.payload_json, j.available_at, \
                    w.generation AS workspace_generation, w.state FROM jobs j \
                    JOIN workspaces w ON w.id = j.workspace_id \
                    AND w.installation_id = j.installation_id \
                    WHERE j.id = $1 AND j.installation_id = $2 \
                    AND j.kind = 'reconcile_workspace' AND j.status = 'pending' \
                    AND j.available_at > $3 \
                    AND w.state IN ('provisioning', 'starting', 'restarting', 'stopping') \
                    AND j.payload_json::jsonb ? 'runtime_wait' \
                    AND j.payload_json::jsonb ->> 'reconciled_generation' = w.generation::text",
                )
                .bind(job_id.to_string())
                .bind(installation_id.as_str())
                .bind(now)
                .fetch_optional(pool)
                .await?;
                row.map(decode_postgres_parked_job)
                    .transpose()
                    .map(Option::flatten)
            }
        }
    }

    /// Only exact, still-pending parks may be accelerated, atomically in one statement.
    pub async fn wake_parked_workspace_jobs(
        &self,
        parked: &[ParkedWorkspaceJob],
        now: i64,
    ) -> Result<HashSet<Uuid>, StorageError> {
        if parked.is_empty() {
            return Ok(HashSet::new());
        }
        let candidates = serde_json::to_string(
            &parked
                .iter()
                .map(|job| {
                    serde_json::json!({
                        "id": job.id,
                        "payload_json": job.payload_json,
                        "until": job.until,
                    })
                })
                .collect::<Vec<_>>(),
        )?;
        let ids: Vec<String> = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query_scalar(
                    "UPDATE jobs SET available_at = ?1, updated_at = ?1 \
                FROM json_each(?3) candidate \
                WHERE jobs.id = json_extract(candidate.value, '$.id') \
                AND jobs.installation_id = ?2 AND jobs.kind = 'reconcile_workspace' \
                AND jobs.status = 'pending' AND jobs.available_at > ?1 \
                AND jobs.payload_json = json_extract(candidate.value, '$.payload_json') \
                AND jobs.available_at = json_extract(candidate.value, '$.until') \
                RETURNING jobs.id",
                )
                .bind(now)
                .bind(installation_id.as_str())
                .bind(&candidates)
                .fetch_all(pool)
                .await?
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query_scalar(
                    "UPDATE jobs SET available_at = $1, updated_at = $1 \
                FROM jsonb_to_recordset($3::jsonb) \
                AS candidate(id text, payload_json text, until bigint) \
                WHERE jobs.id = candidate.id \
                AND jobs.installation_id = $2 AND jobs.kind = 'reconcile_workspace' \
                AND jobs.status = 'pending' AND jobs.available_at > $1 \
                AND jobs.payload_json = candidate.payload_json \
                AND jobs.available_at = candidate.until RETURNING jobs.id",
                )
                .bind(now)
                .bind(installation_id.as_str())
                .bind(&candidates)
                .fetch_all(pool)
                .await?
            }
        };
        ids.into_iter()
            .map(|id| Uuid::parse_str(&id).map_err(StorageError::from))
            .collect()
    }

    pub async fn wake_parked_workspace_job(
        &self,
        parked: &ParkedWorkspaceJob,
        now: i64,
    ) -> Result<bool, StorageError> {
        Ok(self
            .wake_parked_workspace_jobs(std::slice::from_ref(parked), now)
            .await?
            .contains(&parked.id))
    }

    pub async fn defer_job_without_attempt(
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
                "UPDATE jobs SET status = 'pending', available_at = ?1, \
                attempts = CASE WHEN attempts > 0 THEN attempts - 1 ELSE 0 END, \
                lease_owner = NULL, lease_expires_at = NULL, \
                payload_json = CASE WHEN json_type(payload_json, '$.runtime_wait') IS NOT NULL \
                THEN json_remove(payload_json, '$.runtime_wait') ELSE payload_json END, updated_at = ?2 \
                WHERE id = ?3 AND installation_id = ?4 AND status = 'running' \
                AND lease_owner = ?5",
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
                "UPDATE jobs SET status = 'pending', available_at = $1, \
                attempts = CASE WHEN attempts > 0 THEN attempts - 1 ELSE 0 END, \
                lease_owner = NULL, lease_expires_at = NULL, \
                payload_json = CASE WHEN payload_json::jsonb ? 'runtime_wait' \
                THEN (payload_json::jsonb - 'runtime_wait')::text ELSE payload_json END, updated_at = $2 \
                WHERE id = $3 AND installation_id = $4 AND status = 'running' \
                AND lease_owner = $5",
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
