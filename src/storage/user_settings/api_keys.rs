use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;
use sqlx::Row;
use utoipa::ToSchema;
use uuid::Uuid;

use super::super::{Database, StorageError, identity::hash_token};
use crate::auth::ApiKeyScope;

mod persistence;

use persistence::{
    audit_api_key_postgres, audit_api_key_sqlite, ensure_key_capacity_postgres,
    ensure_key_capacity_sqlite, lock_user_postgres, lock_user_sqlite, revoke_postgres,
    revoke_sqlite,
};
pub(in crate::storage) use persistence::{insert_key_postgres, insert_key_sqlite};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ApiKeySummary {
    pub id: Uuid,
    pub name: String,
    pub prefix: String,
    pub last_used_at: Option<i64>,
    pub created_at: i64,
    pub scopes: Vec<ApiKeyScope>,
    pub expires_at: Option<i64>,
}

pub struct CreatedApiKey {
    pub summary: ApiKeySummary,
    pub token: String,
}

impl Database {
    pub async fn list_api_keys(&self, user_id: Uuid) -> Result<Vec<ApiKeySummary>, StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(
                "SELECT id, name, token_prefix, last_used_at, created_at, scopes_json, expires_at FROM user_api_keys WHERE installation_id = ?1 AND user_id = ?2 AND revoked_at IS NULL ORDER BY created_at, id",
            )
            .bind(installation_id.as_str())
            .bind(user_id.to_string())
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(decode_api_key)
            .collect(),
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(
                "SELECT id, name, token_prefix, last_used_at, created_at, scopes_json, expires_at FROM user_api_keys WHERE installation_id = $1 AND user_id = $2 AND revoked_at IS NULL ORDER BY created_at, id",
            )
            .bind(installation_id.as_str())
            .bind(user_id.to_string())
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(decode_api_key)
            .collect(),
        }
    }

    pub async fn create_api_key(
        &self,
        user_id: Uuid,
        name: &str,
        scopes: Vec<ApiKeyScope>,
        expires_at: Option<i64>,
        now: i64,
    ) -> Result<CreatedApiKey, StorageError> {
        let name = validate_api_key_name(name)?;
        let scopes = validate_scopes(scopes)?;
        validate_expiration(expires_at, now)?;
        let token = generate_token()?;
        let summary = ApiKeySummary {
            id: Uuid::now_v7(),
            name,
            prefix: token_prefix(&token),
            last_used_at: None,
            created_at: now,
            scopes,
            expires_at,
        };
        let token_hash = hash_token(&token);
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                lock_user_sqlite(&mut transaction, installation_id.as_str(), user_id).await?;
                ensure_key_capacity_sqlite(&mut transaction, installation_id.as_str(), user_id)
                    .await?;
                insert_key_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    user_id,
                    &summary,
                    &token_hash,
                )
                .await?;
                audit_api_key_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    user_id,
                    summary.id,
                    "user.api_key.create",
                    now,
                )
                .await?;
                transaction.commit().await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                lock_user_postgres(&mut transaction, installation_id.as_str(), user_id).await?;
                ensure_key_capacity_postgres(&mut transaction, installation_id.as_str(), user_id)
                    .await?;
                insert_key_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    user_id,
                    &summary,
                    &token_hash,
                )
                .await?;
                audit_api_key_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    user_id,
                    summary.id,
                    "user.api_key.create",
                    now,
                )
                .await?;
                transaction.commit().await?;
            }
        }
        Ok(CreatedApiKey { summary, token })
    }

    pub async fn revoke_api_key(
        &self,
        user_id: Uuid,
        key_id: Uuid,
        now: i64,
    ) -> Result<(), StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                lock_user_sqlite(&mut transaction, installation_id.as_str(), user_id).await?;
                revoke_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    user_id,
                    key_id,
                    now,
                )
                .await?;
                audit_api_key_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    user_id,
                    key_id,
                    "user.api_key.revoke",
                    now,
                )
                .await?;
                transaction.commit().await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                lock_user_postgres(&mut transaction, installation_id.as_str(), user_id).await?;
                revoke_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    user_id,
                    key_id,
                    now,
                )
                .await?;
                audit_api_key_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    user_id,
                    key_id,
                    "user.api_key.revoke",
                    now,
                )
                .await?;
                transaction.commit().await?;
            }
        }
        Ok(())
    }
}

fn decode_api_key<R: Row>(row: R) -> Result<ApiKeySummary, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    Ok(ApiKeySummary {
        id: Uuid::parse_str(&row.try_get::<String, _>("id")?)?,
        name: row.try_get("name")?,
        prefix: row.try_get("token_prefix")?,
        last_used_at: row.try_get("last_used_at")?,
        created_at: row.try_get("created_at")?,
        scopes: serde_json::from_str(&row.try_get::<String, _>("scopes_json")?)?,
        expires_at: row.try_get("expires_at")?,
    })
}

/// Validate the policy shared by self-service keys and administrator-provisioned
/// initial keys.  New keys must always be explicitly scoped and time-bounded;
/// `Wildcard` remains readable only for keys created before this policy existed.
pub(crate) fn validate_api_key_policy(
    scopes: Vec<ApiKeyScope>,
    expires_at: Option<i64>,
    now: i64,
) -> Result<Vec<ApiKeyScope>, StorageError> {
    let scopes = validate_scopes(scopes)?;
    validate_expiration(expires_at, now)?;
    Ok(scopes)
}

fn validate_scopes(scopes: Vec<ApiKeyScope>) -> Result<Vec<ApiKeyScope>, StorageError> {
    if scopes.is_empty()
        || scopes
            .iter()
            .any(|scope| matches!(scope, ApiKeyScope::Wildcard))
    {
        return Err(StorageError::InvalidApiKey);
    }
    let mut scopes = scopes;
    scopes.sort_by_key(|scope| *scope as u8);
    scopes.dedup();
    Ok(scopes)
}

const MAX_API_KEY_LIFETIME_SECONDS: i64 = 365 * 24 * 60 * 60;

fn validate_expiration(expires_at: Option<i64>, now: i64) -> Result<(), StorageError> {
    let Some(expires_at) = expires_at else {
        return Err(StorageError::InvalidApiKey);
    };
    if expires_at <= now || expires_at.saturating_sub(now) > MAX_API_KEY_LIFETIME_SECONDS {
        return Err(StorageError::InvalidApiKey);
    }
    Ok(())
}

fn validate_api_key_name(name: &str) -> Result<String, StorageError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err(StorageError::InvalidApiKey);
    }
    Ok(name.to_owned())
}

fn generate_token() -> Result<String, StorageError> {
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random).map_err(|_| StorageError::RandomSource)?;
    Ok(format!("mwc_{}", URL_SAFE_NO_PAD.encode(random)))
}

pub(in crate::storage) fn token_prefix(token: &str) -> String {
    let visible = token.chars().take(12).collect::<String>();
    format!("{visible}…")
}
