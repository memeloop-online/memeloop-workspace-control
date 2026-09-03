use sqlx::{PgConnection, SqliteConnection};
use uuid::Uuid;

use super::ApiKeySummary;
use crate::storage::StorageError;

const MAX_ACTIVE_API_KEYS: i64 = 20;

pub(super) async fn lock_user_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    user_id: Uuid,
) -> Result<(), StorageError> {
    let found = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM users WHERE installation_id = ?1 AND id = ?2 AND disabled = 0",
    )
    .bind(installation)
    .bind(user_id.to_string())
    .fetch_one(connection)
    .await?;
    if found != 1 {
        return Err(StorageError::UserNotFound);
    }
    Ok(())
}

pub(super) async fn lock_user_postgres(
    connection: &mut PgConnection,
    installation: &str,
    user_id: Uuid,
) -> Result<(), StorageError> {
    let found = sqlx::query_scalar::<_, String>(
        "SELECT id FROM users WHERE installation_id = $1 AND id = $2 AND disabled = 0 FOR UPDATE",
    )
    .bind(installation)
    .bind(user_id.to_string())
    .fetch_optional(connection)
    .await?;
    if found.is_none() {
        return Err(StorageError::UserNotFound);
    }
    Ok(())
}

pub(super) async fn ensure_key_capacity_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    user_id: Uuid,
) -> Result<(), StorageError> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM user_api_keys WHERE installation_id = ?1 AND user_id = ?2 AND revoked_at IS NULL",
    )
    .bind(installation)
    .bind(user_id.to_string())
    .fetch_one(connection)
    .await?;
    ensure_key_capacity(count)
}

pub(super) async fn ensure_key_capacity_postgres(
    connection: &mut PgConnection,
    installation: &str,
    user_id: Uuid,
) -> Result<(), StorageError> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM user_api_keys WHERE installation_id = $1 AND user_id = $2 AND revoked_at IS NULL",
    )
    .bind(installation)
    .bind(user_id.to_string())
    .fetch_one(connection)
    .await?;
    ensure_key_capacity(count)
}

fn ensure_key_capacity(count: i64) -> Result<(), StorageError> {
    if count >= MAX_ACTIVE_API_KEYS {
        return Err(StorageError::TooManyApiKeys);
    }
    Ok(())
}

pub(in crate::storage) async fn insert_key_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    user_id: Uuid,
    key: &ApiKeySummary,
    token_hash: &str,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO user_api_keys (id, installation_id, user_id, name, token_prefix, token_hash, last_used_at, created_at, revoked_at, scopes_json, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, NULL, ?8, ?9)")
        .bind(key.id.to_string()).bind(installation).bind(user_id.to_string()).bind(&key.name)
        .bind(&key.prefix).bind(token_hash).bind(key.created_at).bind(serde_json::to_string(&key.scopes)?).bind(key.expires_at).execute(connection).await?;
    Ok(())
}

pub(in crate::storage) async fn insert_key_postgres(
    connection: &mut PgConnection,
    installation: &str,
    user_id: Uuid,
    key: &ApiKeySummary,
    token_hash: &str,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO user_api_keys (id, installation_id, user_id, name, token_prefix, token_hash, last_used_at, created_at, revoked_at, scopes_json, expires_at) VALUES ($1, $2, $3, $4, $5, $6, NULL, $7, NULL, $8, $9)")
        .bind(key.id.to_string()).bind(installation).bind(user_id.to_string()).bind(&key.name)
        .bind(&key.prefix).bind(token_hash).bind(key.created_at).bind(serde_json::to_string(&key.scopes)?).bind(key.expires_at).execute(connection).await?;
    Ok(())
}
