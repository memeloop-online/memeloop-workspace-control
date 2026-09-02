use serde::Serialize;
use sqlx::Row;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::Role;
use crate::storage::{Database, Organization, StorageError};

mod organization_locks;
mod organizations;
mod pagination;
mod quotas;
mod users;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UserSummary {
    pub id: Uuid,
    pub display_name: String,
    pub system_admin: bool,
    pub disabled: bool,
    pub created_at: i64,
    /// Present only when the user page was scoped to an organization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub membership_role: Option<Option<Role>>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UserPage {
    pub items: Vec<UserSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OrganizationPage {
    pub items: Vec<Organization>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MembershipSummary {
    pub user: UserSummary,
    pub role: Role,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MembershipPage {
    pub items: Vec<MembershipSummary>,
    pub next_cursor: Option<String>,
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
                    .bind(id)
                    .bind(installation_id.as_str())
                    .bind(actor)
                    .bind(organization)
                    .bind(workspace)
                    .bind(action)
                    .bind(metadata)
                    .bind(now)
                    .execute(pool)
                    .await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)")
                    .bind(id)
                    .bind(installation_id.as_str())
                    .bind(actor)
                    .bind(organization)
                    .bind(workspace)
                    .bind(action)
                    .bind(metadata)
                    .bind(now)
                    .execute(pool)
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn list_audit(
        &self,
        organization_id: Uuid,
        limit: u32,
    ) -> Result<Vec<AuditRecord>, StorageError> {
        let limit = i64::from(limit.clamp(1, 1_000));
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query("SELECT a.id, a.actor_user_id, u.display_name AS actor_display_name, a.organization_id, a.workspace_id, w.name AS workspace_name, w.short_id AS workspace_short_id, a.action, a.metadata_json, a.created_at FROM audit_log a LEFT JOIN users u ON u.installation_id = a.installation_id AND u.id = a.actor_user_id LEFT JOIN workspaces w ON w.installation_id = a.installation_id AND w.id = a.workspace_id WHERE a.installation_id = ?1 AND a.organization_id = ?2 ORDER BY a.created_at DESC, a.id DESC LIMIT ?3")
                .bind(installation_id.as_str())
                .bind(organization_id.to_string())
                .bind(limit)
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(decode_audit)
                .collect(),
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query("SELECT a.id, a.actor_user_id, u.display_name AS actor_display_name, a.organization_id, a.workspace_id, w.name AS workspace_name, w.short_id AS workspace_short_id, a.action, a.metadata_json, a.created_at FROM audit_log a LEFT JOIN users u ON u.installation_id = a.installation_id AND u.id = a.actor_user_id LEFT JOIN workspaces w ON w.installation_id = a.installation_id AND w.id = a.workspace_id WHERE a.installation_id = $1 AND a.organization_id = $2 ORDER BY a.created_at DESC, a.id DESC LIMIT $3")
                .bind(installation_id.as_str())
                .bind(organization_id.to_string())
                .bind(limit)
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(decode_audit)
                .collect(),
        }
    }
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
