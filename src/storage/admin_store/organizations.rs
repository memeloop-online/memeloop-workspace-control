use sqlx::{Row, postgres::PgRow, sqlite::SqliteRow};
use uuid::Uuid;

use crate::auth::Role;
use crate::storage::{Database, Organization, StorageError};

use super::pagination::{decode_cursor, page_limit, page_members, page_organizations};
use super::{MembershipPage, MembershipSummary, OrganizationPage};

impl Database {
    pub async fn rename_organization(
        &self,
        organization_id: Uuid,
        name: &str,
    ) -> Result<Organization, StorageError> {
        let name = name.trim();
        if name.is_empty() || name.len() > 120 {
            return Err(StorageError::InvalidWorkspace);
        }
        let updated = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(
                "UPDATE organizations SET name = ?1 WHERE installation_id = ?2 AND id = ?3",
            )
            .bind(name)
            .bind(installation_id.as_str())
            .bind(organization_id.to_string())
            .execute(pool)
            .await?
            .rows_affected(),
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(
                "UPDATE organizations SET name = $1 WHERE installation_id = $2 AND id = $3",
            )
            .bind(name)
            .bind(installation_id.as_str())
            .bind(organization_id.to_string())
            .execute(pool)
            .await?
            .rows_affected(),
        };
        if updated == 0 {
            return Err(StorageError::OrganizationNotFound);
        }
        self.get_organization(organization_id).await
    }

    pub async fn delete_organization_if_empty(
        &self,
        organization_id: Uuid,
    ) -> Result<(), StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let count: i64 = sqlx::query_scalar("SELECT (SELECT COUNT(*) FROM workspaces WHERE installation_id = ?1 AND organization_id = ?2 AND state <> 'deleted') + (SELECT COUNT(*) FROM workspace_templates WHERE installation_id = ?1 AND organization_id = ?2)")
                    .bind(installation_id.as_str())
                    .bind(organization_id.to_string())
                    .fetch_one(pool)
                    .await?;
                if count > 0 {
                    return Err(StorageError::OrganizationInUse);
                }
                let deleted =
                    sqlx::query("DELETE FROM organizations WHERE installation_id = ?1 AND id = ?2")
                        .bind(installation_id.as_str())
                        .bind(organization_id.to_string())
                        .execute(pool)
                        .await?
                        .rows_affected();
                if deleted == 0 {
                    return Err(StorageError::OrganizationNotFound);
                }
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut tx = pool.begin().await?;
                let count: i64 = sqlx::query_scalar("SELECT (SELECT COUNT(*) FROM workspaces WHERE installation_id = $1 AND organization_id = $2 AND state <> 'deleted') + (SELECT COUNT(*) FROM workspace_templates WHERE installation_id = $1 AND organization_id = $2)")
                    .bind(installation_id.as_str())
                    .bind(organization_id.to_string())
                    .fetch_one(&mut *tx)
                    .await?;
                if count > 0 {
                    return Err(StorageError::OrganizationInUse);
                }
                let deleted =
                    sqlx::query("DELETE FROM organizations WHERE installation_id = $1 AND id = $2")
                        .bind(installation_id.as_str())
                        .bind(organization_id.to_string())
                        .execute(&mut *tx)
                        .await?
                        .rows_affected();
                if deleted == 0 {
                    return Err(StorageError::OrganizationNotFound);
                }
                tx.commit().await?;
            }
        }
        Ok(())
    }

    async fn get_organization(&self, organization_id: Uuid) -> Result<Organization, StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query("SELECT id, name, created_at FROM organizations WHERE installation_id = ?1 AND id = ?2")
                .bind(installation_id.as_str())
                .bind(organization_id.to_string())
                .fetch_optional(pool)
                .await?
                .map(decode_organization)
                .transpose()?
                .ok_or(StorageError::OrganizationNotFound),
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query("SELECT id, name, created_at FROM organizations WHERE installation_id = $1 AND id = $2")
                .bind(installation_id.as_str())
                .bind(organization_id.to_string())
                .fetch_optional(pool)
                .await?
                .map(decode_organization)
                .transpose()?
                .ok_or(StorageError::OrganizationNotFound),
        }
    }

    pub async fn list_organizations_for(
        &self,
        user_id: Uuid,
        system_admin: bool,
    ) -> Result<Vec<Organization>, StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let rows = if system_admin {
                    sqlx::query("SELECT id, name, created_at FROM organizations WHERE installation_id = ?1 ORDER BY name, id")
                        .bind(installation_id.as_str())
                        .fetch_all(pool)
                        .await?
                } else {
                    sqlx::query("SELECT id, name, created_at FROM organizations WHERE installation_id = ?1 AND id IN (SELECT organization_id FROM organization_memberships WHERE installation_id = ?1 AND user_id = ?2) ORDER BY name, id")
                        .bind(installation_id.as_str())
                        .bind(user_id.to_string())
                        .fetch_all(pool)
                        .await?
                };
                rows.into_iter().map(decode_organization).collect()
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let rows = if system_admin {
                    sqlx::query("SELECT id, name, created_at FROM organizations WHERE installation_id = $1 ORDER BY name, id")
                        .bind(installation_id.as_str())
                        .fetch_all(pool)
                        .await?
                } else {
                    sqlx::query("SELECT id, name, created_at FROM organizations WHERE installation_id = $1 AND id IN (SELECT organization_id FROM organization_memberships WHERE installation_id = $1 AND user_id = $2) ORDER BY name, id")
                        .bind(installation_id.as_str())
                        .bind(user_id.to_string())
                        .fetch_all(pool)
                        .await?
                };
                rows.into_iter().map(decode_organization).collect()
            }
        }
    }

    pub async fn list_organizations_page_for(
        &self,
        user_id: Uuid,
        system_admin: bool,
        limit: Option<u32>,
        cursor: Option<&str>,
        search: Option<&str>,
    ) -> Result<OrganizationPage, StorageError> {
        let limit = page_limit(limit);
        let cursor = decode_cursor(cursor)?;
        let search = search.unwrap_or("").trim();
        let pattern = format!("%{search}%");
        let rows = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(
                "SELECT o.id, o.name, o.created_at FROM organizations o WHERE o.installation_id = ?1 \
                 AND (?2 != 0 OR o.id IN (SELECT organization_id FROM organization_memberships WHERE installation_id = ?1 AND user_id = ?3)) \
                 AND (?4 = '' OR o.name LIKE ?5 COLLATE NOCASE) \
                 AND (?6 IS NULL OR o.created_at > ?6 OR (o.created_at = ?6 AND o.id > ?7)) ORDER BY o.created_at, o.id LIMIT ?8",
            )
            .bind(installation_id.as_str())
            .bind(i64::from(system_admin))
            .bind(user_id.to_string())
            .bind(search)
            .bind(&pattern)
            .bind(cursor.as_ref().map(|value| value.created_at))
            .bind(cursor.as_ref().map(|value| value.id.to_string()))
            .bind(limit + 1)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(decode_organization)
            .collect::<Result<Vec<_>, _>>()?,
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(
                "SELECT o.id, o.name, o.created_at FROM organizations o WHERE o.installation_id = $1 \
                 AND ($2 != 0 OR o.id IN (SELECT organization_id FROM organization_memberships WHERE installation_id = $1 AND user_id = $3)) \
                 AND ($4 = '' OR o.name ILIKE $5) \
                 AND ($6 IS NULL OR o.created_at > $6 OR (o.created_at = $6 AND o.id > $7)) ORDER BY o.created_at, o.id LIMIT $8",
            )
            .bind(installation_id.as_str())
            .bind(i64::from(system_admin))
            .bind(user_id.to_string())
            .bind(search)
            .bind(&pattern)
            .bind(cursor.as_ref().map(|value| value.created_at))
            .bind(cursor.as_ref().map(|value| value.id.to_string()))
            .bind(limit + 1)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(decode_organization)
            .collect::<Result<Vec<_>, _>>()?,
        };
        page_organizations(rows, limit)
    }

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
                sqlx::query("INSERT INTO organization_memberships (installation_id, organization_id, user_id, role, created_at) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (installation_id, organization_id, user_id) DO UPDATE SET role = excluded.role")
                    .bind(installation_id.as_str())
                    .bind(organization_id.to_string())
                    .bind(user_id.to_string())
                    .bind(role.as_str())
                    .bind(now)
                    .execute(pool)
                    .await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO organization_memberships (installation_id, organization_id, user_id, role, created_at) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (installation_id, organization_id, user_id) DO UPDATE SET role = excluded.role")
                    .bind(installation_id.as_str())
                    .bind(organization_id.to_string())
                    .bind(user_id.to_string())
                    .bind(role.as_str())
                    .bind(now)
                    .execute(pool)
                    .await?;
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
                sqlx::query("DELETE FROM organization_memberships WHERE installation_id = ?1 AND organization_id = ?2 AND user_id = ?3")
                    .bind(installation_id.as_str())
                    .bind(organization_id.to_string())
                    .bind(user_id.to_string())
                    .execute(pool)
                    .await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query("DELETE FROM organization_memberships WHERE installation_id = $1 AND organization_id = $2 AND user_id = $3")
                    .bind(installation_id.as_str())
                    .bind(organization_id.to_string())
                    .bind(user_id.to_string())
                    .execute(pool)
                    .await?;
            }
        }
        Ok(())
    }
}

fn decode_sqlite_membership(row: SqliteRow) -> Result<MembershipSummary, StorageError> {
    decode_membership(&row)
}

fn decode_postgres_membership(row: PgRow) -> Result<MembershipSummary, StorageError> {
    decode_membership(&row)
}

fn decode_organization<R: Row>(row: R) -> Result<Organization, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    Ok(Organization {
        id: Uuid::parse_str(&row.try_get::<String, _>("id")?)?,
        name: row.try_get("name")?,
        created_at: row.try_get("created_at")?,
    })
}

fn decode_membership<R: Row>(row: &R) -> Result<MembershipSummary, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    let role = row.try_get::<String, _>("role")?;
    Ok(MembershipSummary {
        user: super::users::decode_user(row)?,
        role: Role::from_database(&role).ok_or(StorageError::UnknownRole(role))?,
    })
}
