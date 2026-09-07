use sqlx::{Row, postgres::PgRow, sqlite::SqliteRow};
use uuid::Uuid;

use crate::auth::Role;
use crate::storage::{Database, StorageError};

use super::organization_locks::lock_organization_membership_writes_postgres;
use super::pagination::{decode_cursor, page_limit, page_members};
use super::{MembershipPage, MembershipSummary};

impl Database {
    pub async fn list_members_page(
        &self,
        organization_id: Uuid,
        limit: Option<u32>,
        cursor: Option<&str>,
        search: Option<&str>,
    ) -> Result<MembershipPage, StorageError> {
        let limit = page_limit(limit);
        let cursor = decode_cursor(cursor)?;
        let search = search.unwrap_or("").trim();
        let pattern = format!("%{search}%");
        let rows = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(
                "SELECT u.id, u.display_name, u.system_admin, u.disabled, u.created_at, m.role \
                 FROM organization_memberships m JOIN users u ON u.installation_id = m.installation_id AND u.id = m.user_id \
                 WHERE m.installation_id = ?1 AND m.organization_id = ?2 AND (?3 = '' OR u.display_name LIKE ?4 COLLATE NOCASE) \
                 AND (?5 IS NULL OR u.created_at > ?5 OR (u.created_at = ?5 AND u.id > ?6)) ORDER BY u.created_at, u.id LIMIT ?7",
            )
            .bind(installation_id.as_str())
            .bind(organization_id.to_string())
            .bind(search)
            .bind(&pattern)
            .bind(cursor.as_ref().map(|value| value.created_at))
            .bind(cursor.as_ref().map(|value| value.id.to_string()))
            .bind(limit + 1)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(decode_sqlite_membership)
            .collect::<Result<Vec<_>, _>>()?,
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(
                "SELECT u.id, u.display_name, u.system_admin, u.disabled, u.created_at, m.role \
                 FROM organization_memberships m JOIN users u ON u.installation_id = m.installation_id AND u.id = m.user_id \
                 WHERE m.installation_id = $1 AND m.organization_id = $2 AND ($3 = '' OR u.display_name ILIKE $4) \
                 AND ($5 IS NULL OR u.created_at > $5 OR (u.created_at = $5 AND u.id > $6)) ORDER BY u.created_at, u.id LIMIT $7",
            )
            .bind(installation_id.as_str())
            .bind(organization_id.to_string())
            .bind(search)
            .bind(&pattern)
            .bind(cursor.as_ref().map(|value| value.created_at))
            .bind(cursor.as_ref().map(|value| value.id.to_string()))
            .bind(limit + 1)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(decode_postgres_membership)
            .collect::<Result<Vec<_>, _>>()?,
        };
        page_members(rows, limit)
    }

    pub async fn upsert_membership(
        &self,
        organization_id: Uuid,
        user_id: Uuid,
        role: Role,
        now: i64,
    ) -> Result<(), StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                // IMMEDIATE makes the read/guard/write sequence hold the SQLite write lock
                // before checking the current administrator count. This prevents two
                // concurrent demotions from both observing the same last administrator.
                let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
                let current_role: Option<String> = sqlx::query_scalar(
                    "SELECT role FROM organization_memberships WHERE installation_id = ?1 AND organization_id = ?2 AND user_id = ?3",
                )
                .bind(installation_id.as_str())
                .bind(organization_id.to_string())
                .bind(user_id.to_string())
                .fetch_optional(&mut *tx)
                .await?;
                if current_role.as_deref() == Some(Role::OrganizationAdmin.as_str())
                    && role != Role::OrganizationAdmin
                {
                    ensure_another_organization_admin_sqlite(
                        &mut tx,
                        installation_id.as_str(),
                        organization_id,
                    )
                    .await?;
                }
                sqlx::query("INSERT INTO organization_memberships (installation_id, organization_id, user_id, role, created_at) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (installation_id, organization_id, user_id) DO UPDATE SET role = excluded.role")
                    .bind(installation_id.as_str())
                    .bind(organization_id.to_string())
                    .bind(user_id.to_string())
                    .bind(role.as_str())
                    .bind(now)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut tx = pool.begin().await?;
                lock_organization_membership_writes_postgres(
                    &mut tx,
                    installation_id.as_str(),
                    organization_id,
                )
                .await?;
                let current_role: Option<String> = sqlx::query_scalar(
                    "SELECT role FROM organization_memberships WHERE installation_id = $1 AND organization_id = $2 AND user_id = $3 FOR UPDATE",
                )
                .bind(installation_id.as_str())
                .bind(organization_id.to_string())
                .bind(user_id.to_string())
                .fetch_optional(&mut *tx)
                .await?;
                if current_role.as_deref() == Some(Role::OrganizationAdmin.as_str())
                    && role != Role::OrganizationAdmin
                {
                    ensure_another_organization_admin_postgres(
                        &mut tx,
                        installation_id.as_str(),
                        organization_id,
                    )
                    .await?;
                }
                sqlx::query("INSERT INTO organization_memberships (installation_id, organization_id, user_id, role, created_at) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (installation_id, organization_id, user_id) DO UPDATE SET role = excluded.role")
                    .bind(installation_id.as_str())
                    .bind(organization_id.to_string())
                    .bind(user_id.to_string())
                    .bind(role.as_str())
                    .bind(now)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
            }
        }
        Ok(())
    }

    pub async fn remove_membership(
        &self,
        organization_id: Uuid,
        user_id: Uuid,
    ) -> Result<(), StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
                let current_role: Option<String> = sqlx::query_scalar(
                    "SELECT role FROM organization_memberships WHERE installation_id = ?1 AND organization_id = ?2 AND user_id = ?3",
                )
                .bind(installation_id.as_str())
                .bind(organization_id.to_string())
                .bind(user_id.to_string())
                .fetch_optional(&mut *tx)
                .await?;
                if current_role.as_deref() == Some(Role::OrganizationAdmin.as_str()) {
                    ensure_another_organization_admin_sqlite(
                        &mut tx,
                        installation_id.as_str(),
                        organization_id,
                    )
                    .await?;
                }
                sqlx::query("DELETE FROM organization_memberships WHERE installation_id = ?1 AND organization_id = ?2 AND user_id = ?3")
                    .bind(installation_id.as_str())
                    .bind(organization_id.to_string())
                    .bind(user_id.to_string())
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut tx = pool.begin().await?;
                lock_organization_membership_writes_postgres(
                    &mut tx,
                    installation_id.as_str(),
                    organization_id,
                )
                .await?;
                let current_role: Option<String> = sqlx::query_scalar(
                    "SELECT role FROM organization_memberships WHERE installation_id = $1 AND organization_id = $2 AND user_id = $3 FOR UPDATE",
                )
                .bind(installation_id.as_str())
                .bind(organization_id.to_string())
                .bind(user_id.to_string())
                .fetch_optional(&mut *tx)
                .await?;
                if current_role.as_deref() == Some(Role::OrganizationAdmin.as_str()) {
                    ensure_another_organization_admin_postgres(
                        &mut tx,
                        installation_id.as_str(),
                        organization_id,
                    )
                    .await?;
                }
                sqlx::query("DELETE FROM organization_memberships WHERE installation_id = $1 AND organization_id = $2 AND user_id = $3")
                    .bind(installation_id.as_str())
                    .bind(organization_id.to_string())
                    .bind(user_id.to_string())
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
            }
        }
        Ok(())
    }
}

async fn ensure_another_organization_admin_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    installation_id: &str,
    organization_id: Uuid,
) -> Result<(), StorageError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) \
         FROM organization_memberships membership \
         JOIN users member_user \
           ON member_user.installation_id = membership.installation_id \
          AND member_user.id = membership.user_id \
         WHERE membership.installation_id = ?1 \
           AND membership.organization_id = ?2 \
           AND membership.role = 'organization_admin' \
           AND member_user.disabled = 0",
    )
    .bind(installation_id)
    .bind(organization_id.to_string())
    .fetch_one(&mut **tx)
    .await?;
    if count <= 1 {
        return Err(StorageError::LastOrganizationAdmin);
    }
    Ok(())
}

async fn ensure_another_organization_admin_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    installation_id: &str,
    organization_id: Uuid,
) -> Result<(), StorageError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) \
         FROM organization_memberships membership \
         JOIN users member_user \
           ON member_user.installation_id = membership.installation_id \
          AND member_user.id = membership.user_id \
         WHERE membership.installation_id = $1 \
           AND membership.organization_id = $2 \
           AND membership.role = 'organization_admin' \
           AND member_user.disabled = 0",
    )
    .bind(installation_id)
    .bind(organization_id.to_string())
    .fetch_one(&mut **tx)
    .await?;
    if count <= 1 {
        return Err(StorageError::LastOrganizationAdmin);
    }
    Ok(())
}

fn decode_sqlite_membership(row: SqliteRow) -> Result<MembershipSummary, StorageError> {
    decode_membership(&row)
}

fn decode_postgres_membership(row: PgRow) -> Result<MembershipSummary, StorageError> {
    decode_membership(&row)
}

fn decode_membership<R: Row>(row: &R) -> Result<MembershipSummary, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    let role = row.try_get::<String, _>("role")?;
    Ok(MembershipSummary {
        user: super::user_updates::decode_user(row)?,
        role: Role::from_database(&role).ok_or(StorageError::UnknownRole(role))?,
    })
}
