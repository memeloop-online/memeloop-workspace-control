use sqlx::{PgConnection, SqliteConnection};
use uuid::Uuid;

use super::ApiKeyRevokeResult;
use crate::{auth::ApiKeyScope, storage::StorageError};

pub(super) async fn revoke_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    user_id: Uuid,
    key_id: Uuid,
    now: i64,
) -> Result<ApiKeyRevokeResult, StorageError> {
    let key = active_key_details_sqlite(connection, installation, user_id, key_id).await?;
    let active_scopes =
        all_active_key_scopes_sqlite(connection, installation, user_id, now).await?;
    let system_admin = user_is_system_admin_sqlite(connection, installation, user_id).await?;
    if key.expires_at.is_none_or(|expires_at| expires_at > now) {
        ensure_recovery_key_remains(&key.scopes, &active_scopes, system_admin)?;
    }
    let rows = sqlx::query("UPDATE user_api_keys SET revoked_at = ?1 WHERE installation_id = ?2 AND user_id = ?3 AND id = ?4 AND revoked_at IS NULL")
        .bind(now).bind(installation).bind(user_id.to_string()).bind(key_id.to_string())
        .execute(&mut *connection).await?.rows_affected();
    ensure_key_revoked(rows)?;
    Ok(ApiKeyRevokeResult {
        changed: true,
        remaining_active: remaining_usable_key_count_sqlite(connection, installation, user_id, now)
            .await?,
    })
}

pub(super) async fn revoke_postgres(
    connection: &mut PgConnection,
    installation: &str,
    user_id: Uuid,
    key_id: Uuid,
    now: i64,
) -> Result<ApiKeyRevokeResult, StorageError> {
    let key = active_key_details_postgres(connection, installation, user_id, key_id).await?;
    let active_scopes =
        all_active_key_scopes_postgres(connection, installation, user_id, now).await?;
    let system_admin = user_is_system_admin_postgres(connection, installation, user_id).await?;
    if key.expires_at.is_none_or(|expires_at| expires_at > now) {
        ensure_recovery_key_remains(&key.scopes, &active_scopes, system_admin)?;
    }
    let rows = sqlx::query("UPDATE user_api_keys SET revoked_at = $1 WHERE installation_id = $2 AND user_id = $3 AND id = $4 AND revoked_at IS NULL")
        .bind(now).bind(installation).bind(user_id.to_string()).bind(key_id.to_string())
        .execute(&mut *connection).await?.rows_affected();
    ensure_key_revoked(rows)?;
    Ok(ApiKeyRevokeResult {
        changed: true,
        remaining_active: remaining_usable_key_count_postgres(
            connection,
            installation,
            user_id,
            now,
        )
        .await?,
    })
}

pub(super) async fn revoke_user_key_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    user_id: Uuid,
    key_id: Uuid,
    now: i64,
) -> Result<ApiKeyRevokeResult, StorageError> {
    let changed = sqlx::query(
        "UPDATE user_api_keys SET revoked_at = ?1 WHERE installation_id = ?2 AND user_id = ?3 AND id = ?4 AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(installation)
    .bind(user_id.to_string())
    .bind(key_id.to_string())
    .execute(&mut *connection)
    .await?
    .rows_affected()
        == 1;
    Ok(ApiKeyRevokeResult {
        changed,
        remaining_active: remaining_usable_key_count_sqlite(connection, installation, user_id, now)
            .await?,
    })
}

pub(super) async fn revoke_user_key_postgres(
    connection: &mut PgConnection,
    installation: &str,
    user_id: Uuid,
    key_id: Uuid,
    now: i64,
) -> Result<ApiKeyRevokeResult, StorageError> {
    // Disabled users remain valid administrator targets. Lock an existing row
    // without requiring the account to be active.
    sqlx::query_scalar::<_, String>(
        "SELECT id FROM users WHERE installation_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(installation)
    .bind(user_id.to_string())
    .fetch_optional(&mut *connection)
    .await?;
    let changed = sqlx::query(
        "UPDATE user_api_keys SET revoked_at = $1 WHERE installation_id = $2 AND user_id = $3 AND id = $4 AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(installation)
    .bind(user_id.to_string())
    .bind(key_id.to_string())
    .execute(&mut *connection)
    .await?
    .rows_affected()
        == 1;
    Ok(ApiKeyRevokeResult {
        changed,
        remaining_active: remaining_usable_key_count_postgres(
            connection,
            installation,
            user_id,
            now,
        )
        .await?,
    })
}

struct ActiveKeyDetails {
    scopes: Vec<ApiKeyScope>,
    expires_at: Option<i64>,
}

async fn active_key_details_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    user_id: Uuid,
    key_id: Uuid,
) -> Result<ActiveKeyDetails, StorageError> {
    let key = sqlx::query_as::<_, (String, Option<i64>)>(
        "SELECT scopes_json, expires_at FROM user_api_keys WHERE installation_id = ?1 AND user_id = ?2 \
        AND id = ?3 AND revoked_at IS NULL",
    )
    .bind(installation)
    .bind(user_id.to_string())
    .bind(key_id.to_string())
    .fetch_optional(connection)
    .await?;
    decode_active_key(key)
}

async fn active_key_details_postgres(
    connection: &mut PgConnection,
    installation: &str,
    user_id: Uuid,
    key_id: Uuid,
) -> Result<ActiveKeyDetails, StorageError> {
    let key = sqlx::query_as::<_, (String, Option<i64>)>(
        "SELECT scopes_json, expires_at FROM user_api_keys WHERE installation_id = $1 AND user_id = $2 \
        AND id = $3 AND revoked_at IS NULL",
    )
    .bind(installation)
    .bind(user_id.to_string())
    .bind(key_id.to_string())
    .fetch_optional(connection)
    .await?;
    decode_active_key(key)
}

fn decode_active_key(key: Option<(String, Option<i64>)>) -> Result<ActiveKeyDetails, StorageError> {
    let (scopes_json, expires_at) = key.ok_or(StorageError::ApiKeyNotFound)?;
    Ok(ActiveKeyDetails {
        scopes: serde_json::from_str(&scopes_json)?,
        expires_at,
    })
}

async fn all_active_key_scopes_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    user_id: Uuid,
    now: i64,
) -> Result<Vec<Vec<ApiKeyScope>>, StorageError> {
    let scopes = sqlx::query_scalar::<_, String>(
        "SELECT scopes_json FROM user_api_keys WHERE installation_id = ?1 AND user_id = ?2 \
         AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > ?3)",
    )
    .bind(installation)
    .bind(user_id.to_string())
    .bind(now)
    .fetch_all(connection)
    .await?;
    decode_scopes(scopes)
}

async fn all_active_key_scopes_postgres(
    connection: &mut PgConnection,
    installation: &str,
    user_id: Uuid,
    now: i64,
) -> Result<Vec<Vec<ApiKeyScope>>, StorageError> {
    let scopes = sqlx::query_scalar::<_, String>(
        "SELECT scopes_json FROM user_api_keys WHERE installation_id = $1 AND user_id = $2 \
         AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > $3)",
    )
    .bind(installation)
    .bind(user_id.to_string())
    .bind(now)
    .fetch_all(connection)
    .await?;
    decode_scopes(scopes)
}

fn decode_scopes(values: Vec<String>) -> Result<Vec<Vec<ApiKeyScope>>, StorageError> {
    values
        .into_iter()
        .map(|value| serde_json::from_str(&value).map_err(StorageError::from))
        .collect()
}

async fn user_is_system_admin_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    user_id: Uuid,
) -> Result<bool, StorageError> {
    let value = sqlx::query_scalar::<_, i64>(
        "SELECT system_admin FROM users WHERE installation_id = ?1 AND id = ?2",
    )
    .bind(installation)
    .bind(user_id.to_string())
    .fetch_one(connection)
    .await?;
    Ok(value != 0)
}

async fn user_is_system_admin_postgres(
    connection: &mut PgConnection,
    installation: &str,
    user_id: Uuid,
) -> Result<bool, StorageError> {
    let value = sqlx::query_scalar::<_, i64>(
        "SELECT system_admin FROM users WHERE installation_id = $1 AND id = $2",
    )
    .bind(installation)
    .bind(user_id.to_string())
    .fetch_one(connection)
    .await?;
    Ok(value != 0)
}

async fn remaining_usable_key_count_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    user_id: Uuid,
    now: i64,
) -> Result<usize, StorageError> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM user_api_keys WHERE installation_id = ?1 AND user_id = ?2 \
         AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > ?3)",
    )
    .bind(installation)
    .bind(user_id.to_string())
    .bind(now)
    .fetch_one(connection)
    .await?;
    Ok(count as usize)
}

async fn remaining_usable_key_count_postgres(
    connection: &mut PgConnection,
    installation: &str,
    user_id: Uuid,
    now: i64,
) -> Result<usize, StorageError> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM user_api_keys WHERE installation_id = $1 AND user_id = $2 \
         AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > $3)",
    )
    .bind(installation)
    .bind(user_id.to_string())
    .bind(now)
    .fetch_one(connection)
    .await?;
    Ok(count as usize)
}

fn ensure_recovery_key_remains(
    revoked_key_scopes: &[ApiKeyScope],
    active_key_scopes: &[Vec<ApiKeyScope>],
    system_admin: bool,
) -> Result<(), StorageError> {
    if has_scope(revoked_key_scopes, ApiKeyScope::ManageApiKeys)
        && count_keys_with_scope(active_key_scopes, ApiKeyScope::ManageApiKeys) <= 1
    {
        return Err(StorageError::LastApiKey);
    }
    if system_admin
        && has_scope(revoked_key_scopes, ApiKeyScope::ManageSystem)
        && count_keys_with_scope(active_key_scopes, ApiKeyScope::ManageSystem) <= 1
    {
        return Err(StorageError::LastApiKey);
    }
    Ok(())
}

fn count_keys_with_scope(keys: &[Vec<ApiKeyScope>], required: ApiKeyScope) -> usize {
    keys.iter()
        .filter(|scopes| has_scope(scopes, required))
        .count()
}

fn has_scope(scopes: &[ApiKeyScope], required: ApiKeyScope) -> bool {
    scopes
        .iter()
        .any(|scope| matches!(scope, ApiKeyScope::Wildcard) || *scope == required)
}

fn ensure_key_revoked(rows: u64) -> Result<(), StorageError> {
    if rows != 1 {
        return Err(StorageError::ApiKeyNotFound);
    }
    Ok(())
}
