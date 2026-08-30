use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{AuditRecord, Database, StorageError, admin_store::decode_audit};

#[derive(Debug, Clone)]
pub struct AuditFilter {
    pub organization_id: Option<Uuid>,
    pub limit: u32,
    pub offset: u64,
    pub action: Option<String>,
    pub actor: Option<String>,
    pub workspace: Option<String>,
    pub query: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AuditPage {
    pub items: Vec<AuditRecord>,
    pub next_offset: Option<u64>,
}

impl Database {
    pub async fn page_audit(&self, filter: AuditFilter) -> Result<AuditPage, StorageError> {
        let filter = ValidatedFilter::new(filter)?;
        let rows = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(SQLITE_PAGE)
                .bind(installation_id.as_str())
                .bind(filter.organization_id.map(|value| value.to_string()))
                .bind(filter.action.as_deref())
                .bind(filter.actor.as_deref())
                .bind(filter.actor_like.as_deref())
                .bind(filter.workspace.as_deref())
                .bind(filter.workspace_like.as_deref())
                .bind(filter.query_like.as_deref())
                .bind(filter.fetch_limit)
                .bind(filter.offset)
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(decode_audit)
                .collect::<Result<Vec<_>, _>>()?,
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(POSTGRES_PAGE)
                .bind(installation_id.as_str())
                .bind(filter.organization_id.map(|value| value.to_string()))
                .bind(filter.action.as_deref())
                .bind(filter.actor.as_deref())
                .bind(filter.actor_like.as_deref())
                .bind(filter.workspace.as_deref())
                .bind(filter.workspace_like.as_deref())
                .bind(filter.query_like.as_deref())
                .bind(filter.fetch_limit)
                .bind(filter.offset)
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(decode_audit)
                .collect::<Result<Vec<_>, _>>()?,
        };
        filter.page(rows)
    }
}

struct ValidatedFilter {
    organization_id: Option<Uuid>,
    limit: usize,
    offset_u64: u64,
    offset: i64,
    fetch_limit: i64,
    action: Option<String>,
    actor: Option<String>,
    actor_like: Option<String>,
    workspace: Option<String>,
    workspace_like: Option<String>,
    query_like: Option<String>,
}

impl ValidatedFilter {
    fn new(filter: AuditFilter) -> Result<Self, StorageError> {
        let limit = filter.limit.clamp(1, 100) as usize;
        let offset = i64::try_from(filter.offset).map_err(|_| StorageError::InvalidAuditQuery)?;
        let action = normalize_exact(filter.action)?;
        let actor = normalize_exact(filter.actor)?;
        let workspace = normalize_exact(filter.workspace)?;
        let query = normalize_exact(filter.query)?;
        Ok(Self {
            organization_id: filter.organization_id,
            limit,
            offset_u64: filter.offset,
            offset,
            fetch_limit: i64::try_from(limit + 1).map_err(|_| StorageError::InvalidAuditQuery)?,
            actor_like: like_pattern(actor.as_deref()),
            workspace_like: like_pattern(workspace.as_deref()),
            query_like: like_pattern(query.as_deref()),
            action,
            actor,
            workspace,
        })
    }

    fn page(&self, mut items: Vec<AuditRecord>) -> Result<AuditPage, StorageError> {
        let has_more = items.len() > self.limit;
        items.truncate(self.limit);
        let next_offset = if has_more {
            Some(
                self.offset_u64
                    .checked_add(
                        u64::try_from(self.limit).map_err(|_| StorageError::InvalidAuditQuery)?,
                    )
                    .ok_or(StorageError::InvalidAuditQuery)?,
            )
        } else {
            None
        };
        Ok(AuditPage { items, next_offset })
    }
}

fn normalize_exact(value: Option<String>) -> Result<Option<String>, StorageError> {
    value
        .map(|value| {
            let value = value.trim();
            if value.is_empty() {
                return Ok(None);
            }
            if value.chars().count() > 128 || value.chars().any(char::is_control) {
                return Err(StorageError::InvalidAuditQuery);
            }
            Ok(Some(value.to_owned()))
        })
        .transpose()
        .map(Option::flatten)
}

fn like_pattern(value: Option<&str>) -> Option<String> {
    value.map(|value| {
        let escaped = value
            .to_lowercase()
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        format!("%{escaped}%")
    })
}

const SQLITE_PAGE: &str = "SELECT a.id, a.actor_user_id, u.display_name AS actor_display_name, \
    a.organization_id, a.workspace_id, w.name AS workspace_name, w.short_id AS workspace_short_id, \
    a.action, a.metadata_json, a.created_at FROM audit_log a \
    LEFT JOIN users u ON u.installation_id = a.installation_id AND u.id = a.actor_user_id \
    LEFT JOIN workspaces w ON w.installation_id = a.installation_id AND w.id = a.workspace_id \
    WHERE a.installation_id = ?1 AND (?2 IS NULL OR a.organization_id = ?2) \
    AND (?3 IS NULL OR a.action = ?3) \
    AND (?4 IS NULL OR a.actor_user_id = ?4 OR LOWER(COALESCE(u.display_name, '')) LIKE ?5 ESCAPE '\\') \
    AND (?6 IS NULL OR a.workspace_id = ?6 OR LOWER(COALESCE(w.name, '')) LIKE ?7 ESCAPE '\\' \
        OR LOWER(COALESCE(w.short_id, '')) LIKE ?7 ESCAPE '\\') \
    AND (?8 IS NULL OR LOWER(a.action) LIKE ?8 ESCAPE '\\' \
        OR LOWER(COALESCE(u.display_name, '')) LIKE ?8 ESCAPE '\\' \
        OR LOWER(COALESCE(w.name, '')) LIKE ?8 ESCAPE '\\' \
        OR LOWER(COALESCE(w.short_id, '')) LIKE ?8 ESCAPE '\\' \
        OR LOWER(a.metadata_json) LIKE ?8 ESCAPE '\\') \
    ORDER BY a.created_at DESC, a.id DESC LIMIT ?9 OFFSET ?10";

const POSTGRES_PAGE: &str = "SELECT a.id, a.actor_user_id, u.display_name AS actor_display_name, \
    a.organization_id, a.workspace_id, w.name AS workspace_name, w.short_id AS workspace_short_id, \
    a.action, a.metadata_json, a.created_at FROM audit_log a \
    LEFT JOIN users u ON u.installation_id = a.installation_id AND u.id = a.actor_user_id \
    LEFT JOIN workspaces w ON w.installation_id = a.installation_id AND w.id = a.workspace_id \
    WHERE a.installation_id = $1 AND ($2 IS NULL OR a.organization_id = $2) \
    AND ($3 IS NULL OR a.action = $3) \
    AND ($4 IS NULL OR a.actor_user_id = $4 OR LOWER(COALESCE(u.display_name, '')) LIKE $5 ESCAPE '\\') \
    AND ($6 IS NULL OR a.workspace_id = $6 OR LOWER(COALESCE(w.name, '')) LIKE $7 ESCAPE '\\' \
        OR LOWER(COALESCE(w.short_id, '')) LIKE $7 ESCAPE '\\') \
    AND ($8 IS NULL OR LOWER(a.action) LIKE $8 ESCAPE '\\' \
        OR LOWER(COALESCE(u.display_name, '')) LIKE $8 ESCAPE '\\' \
        OR LOWER(COALESCE(w.name, '')) LIKE $8 ESCAPE '\\' \
        OR LOWER(COALESCE(w.short_id, '')) LIKE $8 ESCAPE '\\' \
        OR LOWER(a.metadata_json) LIKE $8 ESCAPE '\\') \
    ORDER BY a.created_at DESC, a.id DESC LIMIT $9 OFFSET $10";
