use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use utoipa::ToSchema;
use uuid::Uuid;

use super::ApiKeySummary;
use crate::storage::{Database, StorageError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyListStatus {
    Active,
    Revoked,
    All,
}

impl ApiKeyListStatus {
    pub(super) const fn database_value(self) -> i64 {
        match self {
            Self::Active => 0,
            Self::Revoked => 1,
            Self::All => 2,
        }
    }
}

impl Database {
    pub async fn list_api_keys(&self, user_id: Uuid) -> Result<Vec<ApiKeySummary>, StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(
                "SELECT id, name, token_prefix, last_used_at, created_at, scopes_json, expires_at, revoked_at FROM user_api_keys WHERE installation_id = ?1 AND user_id = ?2 AND revoked_at IS NULL ORDER BY created_at, id",
            )
            .bind(installation_id.as_str())
            .bind(user_id.to_string())
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(decode)
            .collect(),
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(
                "SELECT id, name, token_prefix, last_used_at, created_at, scopes_json, expires_at, revoked_at FROM user_api_keys WHERE installation_id = $1 AND user_id = $2 AND revoked_at IS NULL ORDER BY created_at, id",
            )
            .bind(installation_id.as_str())
            .bind(user_id.to_string())
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(decode)
            .collect(),
        }
    }

    /// Lists only non-secret summaries for one user. The opaque cursor is
    /// bound to both the user and lifecycle filter, so cursors cannot be reused
    /// across administrator targets or status filters.
    pub async fn list_api_keys_page(
        &self,
        target_user_id: Uuid,
        status: ApiKeyListStatus,
        limit: Option<u32>,
        cursor: Option<&str>,
    ) -> Result<ApiKeyPage, StorageError> {
        let limit = i64::from(limit.unwrap_or(50).clamp(1, 200));
        let cursor = decode_cursor(cursor, target_user_id, status)?;
        let rows = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(
                "SELECT id, name, token_prefix, last_used_at, created_at, scopes_json, expires_at, revoked_at \
                 FROM user_api_keys WHERE installation_id = ?1 AND user_id = ?2 \
                 AND (?3 = 2 OR (?3 = 0 AND revoked_at IS NULL) OR (?3 = 1 AND revoked_at IS NOT NULL)) \
                 AND (?4 IS NULL OR created_at > ?4 OR (created_at = ?4 AND id > ?5)) \
                 ORDER BY created_at, id LIMIT ?6",
            )
            .bind(installation_id.as_str())
            .bind(target_user_id.to_string())
            .bind(status.database_value())
            .bind(cursor.as_ref().map(|value| value.created_at))
            .bind(cursor.as_ref().map(|value| value.id.to_string()))
            .bind(limit + 1)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(decode)
            .collect::<Result<Vec<_>, _>>()?,
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(
                "SELECT id, name, token_prefix, last_used_at, created_at, scopes_json, expires_at, revoked_at \
                 FROM user_api_keys WHERE installation_id = $1 AND user_id = $2 \
                 AND ($3 = 2 OR ($3 = 0 AND revoked_at IS NULL) OR ($3 = 1 AND revoked_at IS NOT NULL)) \
                 AND ($4 IS NULL OR created_at > $4 OR (created_at = $4 AND id > $5)) \
                 ORDER BY created_at, id LIMIT $6",
            )
            .bind(installation_id.as_str())
            .bind(target_user_id.to_string())
            .bind(status.database_value())
            .bind(cursor.as_ref().map(|value| value.created_at))
            .bind(cursor.as_ref().map(|value| value.id.to_string()))
            .bind(limit + 1)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(decode)
            .collect::<Result<Vec<_>, _>>()?,
        };
        page(rows, limit, target_user_id, status)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ApiKeyPage {
    pub items: Vec<ApiKeySummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(super) struct ApiKeyCursor {
    pub(super) created_at: i64,
    pub(super) id: Uuid,
    target_user_id: Uuid,
    status: ApiKeyListStatus,
}

pub(super) fn decode_cursor(
    cursor: Option<&str>,
    target_user_id: Uuid,
    status: ApiKeyListStatus,
) -> Result<Option<ApiKeyCursor>, StorageError> {
    cursor
        .map(|value| {
            let bytes = URL_SAFE_NO_PAD
                .decode(value)
                .map_err(|_| StorageError::InvalidApiKeyQuery)?;
            let decoded: ApiKeyCursor =
                serde_json::from_slice(&bytes).map_err(|_| StorageError::InvalidApiKeyQuery)?;
            (decoded.target_user_id == target_user_id && decoded.status == status)
                .then_some(decoded)
                .ok_or(StorageError::InvalidApiKeyQuery)
        })
        .transpose()
}

pub(super) fn page(
    mut items: Vec<ApiKeySummary>,
    limit: i64,
    target_user_id: Uuid,
    status: ApiKeyListStatus,
) -> Result<ApiKeyPage, StorageError> {
    let next_cursor = if items.len() > limit as usize {
        items.pop();
        let tail = items.last().expect("page contains at least one entry");
        Some(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&ApiKeyCursor {
            created_at: tail.created_at,
            id: tail.id,
            target_user_id,
            status,
        })?))
    } else {
        None
    };
    Ok(ApiKeyPage { items, next_cursor })
}

pub(super) fn decode<R: Row>(row: R) -> Result<ApiKeySummary, StorageError>
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
        revoked_at: row.try_get("revoked_at")?,
    })
}
