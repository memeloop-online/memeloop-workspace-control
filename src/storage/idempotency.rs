use serde::{Deserialize, Serialize};
use sqlx::Row;

use super::{Database, StorageError};

#[derive(Debug, Clone, Copy)]
pub struct IdempotencyCompletion<'a> {
    pub scope: &'a str,
    pub key: &'a str,
    pub request_hash: &'a str,
    pub status_code: u16,
    pub response_json: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdempotencyDecision {
    Reserved,
    InProgress,
    Conflict,
    Replay(IdempotencyReplay),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdempotencyReplay {
    pub status_code: u16,
    pub response_json: String,
}

impl Database {
    pub async fn begin_idempotency(
        &self,
        scope: &str,
        key: &str,
        request_hash: &str,
        now: i64,
        expires_at: i64,
    ) -> Result<IdempotencyDecision, StorageError> {
        validate(scope, key)?;
        let inserted = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query("DELETE FROM idempotency_keys WHERE installation_id = ?1 AND scope = ?2 AND key = ?3 AND expires_at <= ?4")
                    .bind(installation_id.as_str()).bind(scope).bind(key).bind(now)
                    .execute(pool).await?;
                sqlx::query("INSERT INTO idempotency_keys (installation_id, scope, key, request_hash, response_json, status_code, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, '', 0, ?5, ?6) ON CONFLICT (installation_id, scope, key) DO NOTHING")
                    .bind(installation_id.as_str()).bind(scope).bind(key).bind(request_hash)
                    .bind(now).bind(expires_at).execute(pool).await?.rows_affected()
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query("DELETE FROM idempotency_keys WHERE installation_id = $1 AND scope = $2 AND key = $3 AND expires_at <= $4")
                    .bind(installation_id.as_str()).bind(scope).bind(key).bind(now)
                    .execute(pool).await?;
                sqlx::query("INSERT INTO idempotency_keys (installation_id, scope, key, request_hash, response_json, status_code, created_at, expires_at) VALUES ($1, $2, $3, $4, '', 0, $5, $6) ON CONFLICT (installation_id, scope, key) DO NOTHING")
                    .bind(installation_id.as_str()).bind(scope).bind(key).bind(request_hash)
                    .bind(now).bind(expires_at).execute(pool).await?.rows_affected()
            }
        };
        if inserted == 1 {
            return Ok(IdempotencyDecision::Reserved);
        }
        self.read_idempotency(scope, key, request_hash).await
    }

    pub async fn finish_idempotency(
        &self,
        scope: &str,
        key: &str,
        request_hash: &str,
        status_code: u16,
        response_json: &str,
    ) -> Result<(), StorageError> {
        let completion = IdempotencyCompletion {
            scope,
            key,
            request_hash,
            status_code,
            response_json,
        };
        let affected = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => finish_sqlite(pool, installation_id.as_str(), completion).await?,
            Self::Postgres {
                pool,
                installation_id,
            } => finish_postgres(pool, installation_id.as_str(), completion).await?,
        };
        ensure_finished(affected)?;
        Ok(())
    }

    pub async fn abandon_idempotency(
        &self,
        scope: &str,
        key: &str,
        request_hash: &str,
    ) -> Result<(), StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query(
                    "DELETE FROM idempotency_keys WHERE installation_id = ?1 AND scope = ?2 AND key = ?3 AND request_hash = ?4 AND status_code = 0",
                ).bind(installation_id.as_str()).bind(scope).bind(key).bind(request_hash)
                    .execute(pool).await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query(
                    "DELETE FROM idempotency_keys WHERE installation_id = $1 AND scope = $2 AND key = $3 AND request_hash = $4 AND status_code = 0",
                ).bind(installation_id.as_str()).bind(scope).bind(key).bind(request_hash)
                    .execute(pool).await?;
            }
        };
        Ok(())
    }

    async fn read_idempotency(
        &self,
        scope: &str,
        key: &str,
        request_hash: &str,
    ) -> Result<IdempotencyDecision, StorageError> {
        let (stored_hash, status_code, response_json): (String, i64, String) = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let row = sqlx::query("SELECT request_hash, status_code, response_json FROM idempotency_keys WHERE installation_id = ?1 AND scope = ?2 AND key = ?3")
                    .bind(installation_id.as_str()).bind(scope).bind(key).fetch_one(pool).await?;
                (
                    row.try_get("request_hash")?,
                    row.try_get("status_code")?,
                    row.try_get("response_json")?,
                )
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let row = sqlx::query("SELECT request_hash, status_code, response_json FROM idempotency_keys WHERE installation_id = $1 AND scope = $2 AND key = $3")
                    .bind(installation_id.as_str()).bind(scope).bind(key).fetch_one(pool).await?;
                (
                    row.try_get("request_hash")?,
                    row.try_get("status_code")?,
                    row.try_get("response_json")?,
                )
            }
        };
        if stored_hash != request_hash {
            return Ok(IdempotencyDecision::Conflict);
        }
        if status_code == 0 {
            return Ok(IdempotencyDecision::InProgress);
        }
        Ok(IdempotencyDecision::Replay(IdempotencyReplay {
            status_code: u16::try_from(status_code)
                .map_err(|_| StorageError::IdempotencyReservationLost)?,
            response_json,
        }))
    }
}

pub(in crate::storage) async fn finish_sqlite<'e, E>(
    executor: E,
    installation: &str,
    completion: IdempotencyCompletion<'_>,
) -> Result<u64, StorageError>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    Ok(sqlx::query(
        "UPDATE idempotency_keys SET status_code = ?1, response_json = ?2 WHERE \
         installation_id = ?3 AND scope = ?4 AND key = ?5 AND request_hash = ?6 AND status_code = 0",
    )
    .bind(i64::from(completion.status_code))
    .bind(completion.response_json)
    .bind(installation)
    .bind(completion.scope)
    .bind(completion.key)
    .bind(completion.request_hash)
    .execute(executor)
    .await?
    .rows_affected())
}

pub(in crate::storage) async fn finish_postgres<'e, E>(
    executor: E,
    installation: &str,
    completion: IdempotencyCompletion<'_>,
) -> Result<u64, StorageError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    Ok(sqlx::query(
        "UPDATE idempotency_keys SET status_code = $1, response_json = $2 WHERE \
         installation_id = $3 AND scope = $4 AND key = $5 AND request_hash = $6 AND status_code = 0",
    )
    .bind(i64::from(completion.status_code))
    .bind(completion.response_json)
    .bind(installation)
    .bind(completion.scope)
    .bind(completion.key)
    .bind(completion.request_hash)
    .execute(executor)
    .await?
    .rows_affected())
}

pub(in crate::storage) fn ensure_finished(affected: u64) -> Result<(), StorageError> {
    if affected != 1 {
        return Err(StorageError::IdempotencyReservationLost);
    }
    Ok(())
}

fn validate(scope: &str, key: &str) -> Result<(), StorageError> {
    if scope.is_empty() || key.is_empty() || scope.len() > 255 || key.len() > 255 {
        return Err(StorageError::InvalidIdempotencyKey);
    }
    Ok(())
}
