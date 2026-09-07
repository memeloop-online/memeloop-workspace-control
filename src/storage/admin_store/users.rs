use uuid::Uuid;

use crate::storage::{Database, StorageError};

use super::UserPage;
use super::UserSummary;
use super::pagination::{decode_cursor, page_limit, page_users};
use super::user_updates::{
    decode_postgres_user, decode_postgres_user_with_membership, decode_sqlite_user,
    decode_sqlite_user_with_membership, escape_like_pattern, update_user_postgres,
    update_user_sqlite,
};

impl Database {
    pub async fn update_user(
        &self,
        user_id: Uuid,
        display_name: Option<&str>,
        system_admin: Option<bool>,
        disabled: Option<bool>,
    ) -> Result<UserSummary, StorageError> {
        if display_name.is_some_and(|value| value.trim().is_empty() || value.len() > 120) {
            return Err(StorageError::InvalidUserProfile);
        }
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                update_user_sqlite(
                    pool,
                    installation_id.as_str(),
                    user_id,
                    display_name,
                    system_admin,
                    disabled,
                )
                .await
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                update_user_postgres(
                    pool,
                    installation_id.as_str(),
                    user_id,
                    display_name,
                    system_admin,
                    disabled,
                )
                .await
            }
        }
    }

    pub async fn list_users(&self) -> Result<Vec<UserSummary>, StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query("SELECT id, display_name, system_admin, disabled, created_at FROM users WHERE installation_id = ?1 ORDER BY created_at, id")
                .bind(installation_id.as_str())
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(decode_sqlite_user)
                .collect(),
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query("SELECT id, display_name, system_admin, disabled, created_at FROM users WHERE installation_id = $1 ORDER BY created_at, id")
                .bind(installation_id.as_str())
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(decode_postgres_user)
                .collect(),
        }
    }

    pub async fn list_users_page(
        &self,
        limit: Option<u32>,
        cursor: Option<&str>,
        search: Option<&str>,
        organization_id: Option<Uuid>,
    ) -> Result<UserPage, StorageError> {
        let limit = page_limit(limit);
        let cursor = decode_cursor(cursor)?;
        let search = search.unwrap_or("").trim().to_lowercase();
        let pattern = format!("%{}%", escape_like_pattern(&search));
        let rows = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(
                "SELECT u.id, u.display_name, u.system_admin, u.disabled, u.created_at, m.role AS membership_role \
                 FROM users u LEFT JOIN organization_memberships m \
                 ON m.installation_id = u.installation_id AND m.organization_id = ?2 AND m.user_id = u.id \
                 WHERE u.installation_id = ?1 AND (?3 = '' OR LOWER(u.display_name) LIKE ?4 ESCAPE '\\') \
                 AND (?5 IS NULL OR u.created_at > ?5 OR (u.created_at = ?5 AND u.id > ?6)) \
                 ORDER BY u.created_at, u.id LIMIT ?7",
            )
            .bind(installation_id.as_str())
            .bind(organization_id.map(|id| id.to_string()))
            .bind(search)
            .bind(&pattern)
            .bind(cursor.as_ref().map(|value| value.created_at))
            .bind(cursor.as_ref().map(|value| value.id.to_string()))
            .bind(limit + 1)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| {
                if organization_id.is_some() {
                    decode_sqlite_user_with_membership(row)
                } else {
                    decode_sqlite_user(row)
                }
            })
            .collect::<Result<Vec<_>, _>>()?,
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(
                "SELECT u.id, u.display_name, u.system_admin, u.disabled, u.created_at, m.role AS membership_role \
                 FROM users u LEFT JOIN organization_memberships m \
                 ON m.installation_id = u.installation_id AND m.organization_id = $2 AND m.user_id = u.id \
                 WHERE u.installation_id = $1 AND ($3 = '' OR LOWER(u.display_name) LIKE $4 ESCAPE '\\') \
                 AND ($5 IS NULL OR u.created_at > $5 OR (u.created_at = $5 AND u.id > $6)) \
                 ORDER BY u.created_at, u.id LIMIT $7",
            )
            .bind(installation_id.as_str())
            .bind(organization_id.map(|id| id.to_string()))
            .bind(search)
            .bind(&pattern)
            .bind(cursor.as_ref().map(|value| value.created_at))
            .bind(cursor.as_ref().map(|value| value.id.to_string()))
            .bind(limit + 1)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| {
                if organization_id.is_some() {
                    decode_postgres_user_with_membership(row)
                } else {
                    decode_postgres_user(row)
                }
            })
            .collect::<Result<Vec<_>, _>>()?,
        };
        page_users(rows, limit)
    }
}
