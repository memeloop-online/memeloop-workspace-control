use sqlx::{Row, postgres::PgRow, sqlite::SqliteRow};
use uuid::Uuid;

use crate::storage::{Database, StorageError};

use super::UserPage;
use super::UserSummary;
use super::pagination::{decode_cursor, page_limit, page_users};

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
                let mut tx = pool.begin().await?;
                let current = sqlx::query("SELECT id, display_name, system_admin, disabled, created_at FROM users WHERE installation_id = ?1 AND id = ?2")
                    .bind(installation_id.as_str())
                    .bind(user_id.to_string())
                    .fetch_optional(&mut *tx)
                    .await?
                    .map(decode_sqlite_user)
                    .transpose()?
                    .ok_or(StorageError::UserNotFound)?;
                let next_admin = system_admin.unwrap_or(current.system_admin);
                let next_disabled = disabled.unwrap_or(current.disabled);
                if current.system_admin && !current.disabled && (!next_admin || next_disabled) {
                    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE installation_id = ?1 AND system_admin = 1 AND disabled = 0")
                        .bind(installation_id.as_str())
                        .fetch_one(&mut *tx)
                        .await?;
                    if count <= 1 {
                        return Err(StorageError::LastSystemAdmin);
                    }
                }
                sqlx::query("UPDATE users SET display_name = ?1, system_admin = ?2, disabled = ?3 WHERE installation_id = ?4 AND id = ?5")
                    .bind(display_name.unwrap_or(&current.display_name).trim())
                    .bind(i64::from(next_admin))
                    .bind(i64::from(next_disabled))
                    .bind(installation_id.as_str())
                    .bind(user_id.to_string())
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                Ok(UserSummary {
                    display_name: display_name
                        .unwrap_or(&current.display_name)
                        .trim()
                        .to_owned(),
                    system_admin: next_admin,
                    disabled: next_disabled,
                    ..current
                })
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut tx = pool.begin().await?;
                // Serialize all changes that could affect the last active system administrator
                // for this installation. Row locks alone do not protect two concurrent demotions
                // of different administrator rows.
                sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                    .bind(format!("mwc:active-system-admins:{installation_id}"))
                    .execute(&mut *tx)
                    .await?;
                let current = sqlx::query("SELECT id, display_name, system_admin, disabled, created_at FROM users WHERE installation_id = $1 AND id = $2 FOR UPDATE")
                    .bind(installation_id.as_str())
                    .bind(user_id.to_string())
                    .fetch_optional(&mut *tx)
                    .await?
                    .map(decode_postgres_user)
                    .transpose()?
                    .ok_or(StorageError::UserNotFound)?;
                let next_admin = system_admin.unwrap_or(current.system_admin);
                let next_disabled = disabled.unwrap_or(current.disabled);
                if current.system_admin && !current.disabled && (!next_admin || next_disabled) {
                    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE installation_id = $1 AND system_admin = 1 AND disabled = 0")
                        .bind(installation_id.as_str())
                        .fetch_one(&mut *tx)
                        .await?;
                    if count <= 1 {
                        return Err(StorageError::LastSystemAdmin);
                    }
                }
                sqlx::query("UPDATE users SET display_name = $1, system_admin = $2, disabled = $3 WHERE installation_id = $4 AND id = $5")
                    .bind(display_name.unwrap_or(&current.display_name).trim())
                    .bind(i64::from(next_admin))
                    .bind(i64::from(next_disabled))
                    .bind(installation_id.as_str())
                    .bind(user_id.to_string())
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                Ok(UserSummary {
                    display_name: display_name
                        .unwrap_or(&current.display_name)
                        .trim()
                        .to_owned(),
                    system_admin: next_admin,
                    disabled: next_disabled,
                    ..current
                })
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
    ) -> Result<UserPage, StorageError> {
        let limit = page_limit(limit);
        let cursor = decode_cursor(cursor)?;
        let search = search.unwrap_or("").trim();
        let pattern = format!("%{search}%");
        let rows = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(
                "SELECT id, display_name, system_admin, disabled, created_at FROM users \
                 WHERE installation_id = ?1 AND (?2 = '' OR display_name LIKE ?3 COLLATE NOCASE) \
                 AND (?4 IS NULL OR created_at > ?4 OR (created_at = ?4 AND id > ?5)) \
                 ORDER BY created_at, id LIMIT ?6",
            )
            .bind(installation_id.as_str())
            .bind(search)
            .bind(&pattern)
            .bind(cursor.as_ref().map(|value| value.created_at))
            .bind(cursor.as_ref().map(|value| value.id.to_string()))
            .bind(limit + 1)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(decode_sqlite_user)
            .collect::<Result<Vec<_>, _>>()?,
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(
                "SELECT id, display_name, system_admin, disabled, created_at FROM users \
                 WHERE installation_id = $1 AND ($2 = '' OR display_name ILIKE $3) \
                 AND ($4 IS NULL OR created_at > $4 OR (created_at = $4 AND id > $5)) \
                 ORDER BY created_at, id LIMIT $6",
            )
            .bind(installation_id.as_str())
            .bind(search)
            .bind(&pattern)
            .bind(cursor.as_ref().map(|value| value.created_at))
            .bind(cursor.as_ref().map(|value| value.id.to_string()))
            .bind(limit + 1)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(decode_postgres_user)
            .collect::<Result<Vec<_>, _>>()?,
        };
        page_users(rows, limit)
    }
}

fn decode_sqlite_user(row: SqliteRow) -> Result<UserSummary, StorageError> {
    decode_user(&row)
}

fn decode_postgres_user(row: PgRow) -> Result<UserSummary, StorageError> {
    decode_user(&row)
}

pub(super) fn decode_user<R: Row>(row: &R) -> Result<UserSummary, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    Ok(UserSummary {
        id: Uuid::parse_str(&row.try_get::<String, _>("id")?)?,
        display_name: row.try_get("display_name")?,
        system_admin: row.try_get::<i64, _>("system_admin")? != 0,
        disabled: row.try_get::<i64, _>("disabled")? != 0,
        created_at: row.try_get("created_at")?,
    })
}
