use serde::Serialize;
use sqlx::{Row, postgres::PgRow, sqlite::SqliteRow};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{auth::Role, quota::Resources};

use super::{Database, Organization, StorageError};

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UserSummary {
    pub id: Uuid,
    pub display_name: String,
    pub system_admin: bool,
    pub disabled: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AuditRecord {
    pub id: Uuid,
    pub actor_user_id: Option<Uuid>,
    pub actor_display_name: Option<String>,
    pub organization_id: Option<Uuid>,
    pub workspace_id: Option<Uuid>,
    pub workspace_name: Option<String>,
    pub workspace_short_id: Option<String>,
    pub action: String,
    pub metadata: serde_json::Value,
    pub created_at: i64,
}

impl Database {
    pub async fn record_audit(
        &self,
        actor_user_id: Option<Uuid>,
        organization_id: Option<Uuid>,
        workspace_id: Option<Uuid>,
        action: &str,
        metadata: serde_json::Value,
        now: i64,
    ) -> Result<(), StorageError> {
        let id = Uuid::now_v7().to_string();
        let actor = actor_user_id.map(|id| id.to_string());
        let organization = organization_id.map(|id| id.to_string());
        let workspace = workspace_id.map(|id| id.to_string());
        let metadata = serde_json::to_string(&metadata)?;
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)")
                .bind(id).bind(installation_id.as_str()).bind(actor).bind(organization).bind(workspace).bind(action).bind(metadata).bind(now).execute(pool).await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)")
                .bind(id).bind(installation_id.as_str()).bind(actor).bind(organization).bind(workspace).bind(action).bind(metadata).bind(now).execute(pool).await?;
            }
        }
        Ok(())
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
                        .bind(installation_id.as_str()).fetch_all(pool).await?
                } else {
                    sqlx::query("SELECT id, name, created_at FROM organizations WHERE installation_id = ?1 AND id IN (SELECT organization_id FROM organization_memberships WHERE installation_id = ?1 AND user_id = ?2) ORDER BY name, id")
                        .bind(installation_id.as_str()).bind(user_id.to_string()).fetch_all(pool).await?
                };
                rows.into_iter().map(decode_organization).collect()
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let rows = if system_admin {
                    sqlx::query("SELECT id, name, created_at FROM organizations WHERE installation_id = $1 ORDER BY name, id")
                        .bind(installation_id.as_str()).fetch_all(pool).await?
                } else {
                    sqlx::query("SELECT id, name, created_at FROM organizations WHERE installation_id = $1 AND id IN (SELECT organization_id FROM organization_memberships WHERE installation_id = $1 AND user_id = $2) ORDER BY name, id")
                        .bind(installation_id.as_str()).bind(user_id.to_string()).fetch_all(pool).await?
                };
                rows.into_iter().map(decode_organization).collect()
            }
        }
    }

    pub async fn list_users(&self) -> Result<Vec<UserSummary>, StorageError> {
        match self {
            Self::Sqlite { pool, installation_id } => sqlx::query("SELECT id, display_name, system_admin, disabled, created_at FROM users WHERE installation_id = ?1 ORDER BY created_at, id")
                .bind(installation_id.as_str()).fetch_all(pool).await?.into_iter().map(decode_sqlite_user).collect(),
            Self::Postgres { pool, installation_id } => sqlx::query("SELECT id, display_name, system_admin, disabled, created_at FROM users WHERE installation_id = $1 ORDER BY created_at, id")
                .bind(installation_id.as_str()).fetch_all(pool).await?.into_iter().map(decode_postgres_user).collect(),
        }
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
                    .bind(installation_id.as_str()).bind(organization_id.to_string()).bind(user_id.to_string())
                    .bind(role.as_str()).bind(now).execute(pool).await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO organization_memberships (installation_id, organization_id, user_id, role, created_at) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (installation_id, organization_id, user_id) DO UPDATE SET role = excluded.role")
                    .bind(installation_id.as_str()).bind(organization_id.to_string()).bind(user_id.to_string())
                    .bind(role.as_str()).bind(now).execute(pool).await?;
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

    pub async fn get_organization_quota(
        &self,
        organization_id: Uuid,
    ) -> Result<Option<Resources>, StorageError> {
        let row = match self {
            Self::Sqlite { pool, installation_id } => {
                let row = sqlx::query("SELECT cpu_millis, memory_mib, gpu_count, disk_gib FROM organization_quotas WHERE installation_id = ?1 AND organization_id = ?2")
                    .bind(installation_id.as_str()).bind(organization_id.to_string()).fetch_optional(pool).await?;
                return row.map(decode_resources).transpose();
            }
            Self::Postgres { pool, installation_id } => sqlx::query("SELECT cpu_millis, memory_mib, gpu_count, disk_gib FROM organization_quotas WHERE installation_id = $1 AND organization_id = $2")
                .bind(installation_id.as_str()).bind(organization_id.to_string()).fetch_optional(pool).await?,
        };
        row.map(decode_resources).transpose()
    }

    pub async fn set_user_quota(
        &self,
        user_id: Uuid,
        resources: Resources,
        now: i64,
    ) -> Result<(), StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO user_quotas (installation_id, user_id, cpu_millis, memory_mib, gpu_count, disk_gib, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT (installation_id, user_id) DO UPDATE SET cpu_millis = excluded.cpu_millis, memory_mib = excluded.memory_mib, gpu_count = excluded.gpu_count, disk_gib = excluded.disk_gib, updated_at = excluded.updated_at")
                    .bind(installation_id.as_str()).bind(user_id.to_string())
                    .bind(as_i64(resources.cpu_millis)?).bind(as_i64(resources.memory_mib)?)
                    .bind(i64::from(resources.gpu_count)).bind(as_i64(resources.disk_gib)?).bind(now)
                    .execute(pool).await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO user_quotas (installation_id, user_id, cpu_millis, memory_mib, gpu_count, disk_gib, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (installation_id, user_id) DO UPDATE SET cpu_millis = excluded.cpu_millis, memory_mib = excluded.memory_mib, gpu_count = excluded.gpu_count, disk_gib = excluded.disk_gib, updated_at = excluded.updated_at")
                    .bind(installation_id.as_str()).bind(user_id.to_string())
                    .bind(as_i64(resources.cpu_millis)?).bind(as_i64(resources.memory_mib)?)
                    .bind(i64::from(resources.gpu_count)).bind(as_i64(resources.disk_gib)?).bind(now)
                    .execute(pool).await?;
            }
        }
        Ok(())
    }

    pub async fn get_user_quota(&self, user_id: Uuid) -> Result<Option<Resources>, StorageError> {
        let row = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let row = sqlx::query("SELECT cpu_millis, memory_mib, gpu_count, disk_gib FROM user_quotas WHERE installation_id = ?1 AND user_id = ?2")
                    .bind(installation_id.as_str()).bind(user_id.to_string())
                    .fetch_optional(pool).await?;
                return row.map(decode_resources).transpose();
            }
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query("SELECT cpu_millis, memory_mib, gpu_count, disk_gib FROM user_quotas WHERE installation_id = $1 AND user_id = $2")
                .bind(installation_id.as_str()).bind(user_id.to_string())
                .fetch_optional(pool).await?,
        };
        row.map(decode_resources).transpose()
    }

    pub async fn list_audit(
        &self,
        organization_id: Uuid,
        limit: u32,
    ) -> Result<Vec<AuditRecord>, StorageError> {
        let limit = i64::from(limit.clamp(1, 1_000));
        match self {
            Self::Sqlite { pool, installation_id } => sqlx::query("SELECT a.id, a.actor_user_id, u.display_name AS actor_display_name, a.organization_id, a.workspace_id, w.name AS workspace_name, w.short_id AS workspace_short_id, a.action, a.metadata_json, a.created_at FROM audit_log a LEFT JOIN users u ON u.installation_id = a.installation_id AND u.id = a.actor_user_id LEFT JOIN workspaces w ON w.installation_id = a.installation_id AND w.id = a.workspace_id WHERE a.installation_id = ?1 AND a.organization_id = ?2 ORDER BY a.created_at DESC, a.id DESC LIMIT ?3")
                .bind(installation_id.as_str()).bind(organization_id.to_string()).bind(limit).fetch_all(pool).await?
                .into_iter().map(decode_audit).collect(),
            Self::Postgres { pool, installation_id } => sqlx::query("SELECT a.id, a.actor_user_id, u.display_name AS actor_display_name, a.organization_id, a.workspace_id, w.name AS workspace_name, w.short_id AS workspace_short_id, a.action, a.metadata_json, a.created_at FROM audit_log a LEFT JOIN users u ON u.installation_id = a.installation_id AND u.id = a.actor_user_id LEFT JOIN workspaces w ON w.installation_id = a.installation_id AND w.id = a.workspace_id WHERE a.installation_id = $1 AND a.organization_id = $2 ORDER BY a.created_at DESC, a.id DESC LIMIT $3")
                .bind(installation_id.as_str()).bind(organization_id.to_string()).bind(limit).fetch_all(pool).await?
                .into_iter().map(decode_audit).collect(),
        }
    }
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

fn decode_sqlite_user(row: SqliteRow) -> Result<UserSummary, StorageError> {
    decode_user(&row)
}
fn decode_postgres_user(row: PgRow) -> Result<UserSummary, StorageError> {
    decode_user(&row)
}
fn decode_user<R: Row>(row: &R) -> Result<UserSummary, StorageError>
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

fn decode_resources<R: Row>(row: R) -> Result<Resources, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    Ok(Resources {
        cpu_millis: u64::try_from(row.try_get::<i64, _>("cpu_millis")?)
            .map_err(|_| StorageError::InvalidWorkspace)?,
        memory_mib: u64::try_from(row.try_get::<i64, _>("memory_mib")?)
            .map_err(|_| StorageError::InvalidWorkspace)?,
        gpu_count: u32::try_from(row.try_get::<i64, _>("gpu_count")?)
            .map_err(|_| StorageError::InvalidWorkspace)?,
        disk_gib: u64::try_from(row.try_get::<i64, _>("disk_gib")?)
            .map_err(|_| StorageError::InvalidWorkspace)?,
    })
}

pub(super) fn decode_audit<R: Row>(row: R) -> Result<AuditRecord, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    let parse = |value: Option<String>| value.map(|id| Uuid::parse_str(&id)).transpose();
    Ok(AuditRecord {
        id: Uuid::parse_str(&row.try_get::<String, _>("id")?)?,
        actor_user_id: parse(row.try_get("actor_user_id")?)?,
        actor_display_name: row.try_get("actor_display_name")?,
        organization_id: parse(row.try_get("organization_id")?)?,
        workspace_id: parse(row.try_get("workspace_id")?)?,
        workspace_name: row.try_get("workspace_name")?,
        workspace_short_id: row.try_get("workspace_short_id")?,
        action: row.try_get("action")?,
        metadata: serde_json::from_str(&row.try_get::<String, _>("metadata_json")?)?,
        created_at: row.try_get("created_at")?,
    })
}

fn as_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::InvalidWorkspace)
}
