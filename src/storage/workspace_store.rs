use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    crypto::EnvelopeCipher, injections::InjectionItem, quota::Resources, workspaces::Workspace,
};

use super::{Database, StorageError, WorkspaceInjectionRefs};

mod creation;
mod row;

pub(super) use row::{
    decode_postgres, decode_sqlite, select_workspace_by_short_id_sql, select_workspace_sql,
};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateWorkspace {
    pub organization_id: Uuid,
    pub owner_id: Uuid,
    pub name: String,
    pub template_id: Uuid,
    /// Optional per-workspace resource limits. The selected template remains the source of every
    /// other setting, while these final values are copied into the immutable workspace snapshot.
    #[serde(default)]
    pub resources: Option<Resources>,
    #[serde(default)]
    pub organization_injection_refs: Option<Vec<String>>,
    #[serde(default)]
    pub user_injection_refs: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WorkspacePage {
    pub items: Vec<Workspace>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct WorkspaceCursor {
    created_at: i64,
    id: Uuid,
}

impl Database {
    pub async fn list_workspaces_by_ids(
        &self,
        organization_id: Uuid,
        workspace_ids: &[Uuid],
    ) -> Result<Vec<Workspace>, StorageError> {
        if workspace_ids.is_empty() {
            return Ok(Vec::new());
        }
        if workspace_ids.len() > 100 {
            return Err(StorageError::InvalidWorkspace);
        }
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let placeholders = vec!["?"; workspace_ids.len()].join(",");
                let sql = format!(
                    "SELECT {} FROM workspaces WHERE installation_id = ? AND organization_id = ? AND state <> 'deleted' AND id IN ({placeholders}) ORDER BY created_at, id",
                    row::WORKSPACE_COLUMNS
                );
                let mut query = sqlx::query(&sql)
                    .bind(installation_id.as_str())
                    .bind(organization_id.to_string());
                for id in workspace_ids {
                    query = query.bind(id.to_string());
                }
                query
                    .fetch_all(pool)
                    .await?
                    .into_iter()
                    .map(decode_sqlite)
                    .collect()
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let placeholders = (3..workspace_ids.len() + 3)
                    .map(|index| format!("${index}"))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {} FROM workspaces WHERE installation_id = $1 AND organization_id = $2 AND state <> 'deleted' AND id IN ({placeholders}) ORDER BY created_at, id",
                    row::WORKSPACE_COLUMNS
                );
                let mut query = sqlx::query(&sql)
                    .bind(installation_id.as_str())
                    .bind(organization_id.to_string());
                for id in workspace_ids {
                    query = query.bind(id.to_string());
                }
                query
                    .fetch_all(pool)
                    .await?
                    .into_iter()
                    .map(decode_postgres)
                    .collect()
            }
        }
    }

    pub async fn create_workspace(
        &self,
        command: CreateWorkspace,
        allow_cluster_access: bool,
        actor_user_id: Uuid,
        now: i64,
    ) -> Result<Workspace, StorageError> {
        self.create_workspace_inner(
            command,
            None,
            None,
            allow_cluster_access,
            actor_user_id,
            now,
        )
        .await
    }

    pub async fn create_workspace_with_inline_injections(
        &self,
        command: CreateWorkspace,
        cipher: &EnvelopeCipher,
        inline: &[InjectionItem],
        allow_cluster_access: bool,
        actor_user_id: Uuid,
        now: i64,
    ) -> Result<Workspace, StorageError> {
        self.create_workspace_inner(
            command,
            Some((cipher, inline)),
            None,
            allow_cluster_access,
            actor_user_id,
            now,
        )
        .await
    }

    pub async fn create_workspace_with_admitted_template(
        &self,
        command: CreateWorkspace,
        inline: Option<(&EnvelopeCipher, &[InjectionItem])>,
        admitted_template_yaml: &str,
        allow_cluster_access: bool,
        actor_user_id: Uuid,
        now: i64,
    ) -> Result<Workspace, StorageError> {
        self.create_workspace_inner(
            command,
            inline,
            Some(admitted_template_yaml),
            allow_cluster_access,
            actor_user_id,
            now,
        )
        .await
    }

    async fn create_workspace_inner(
        &self,
        command: CreateWorkspace,
        inline: Option<(&EnvelopeCipher, &[InjectionItem])>,
        admitted_template_yaml: Option<&str>,
        allow_cluster_access: bool,
        actor_user_id: Uuid,
        now: i64,
    ) -> Result<Workspace, StorageError> {
        if command.name.trim().is_empty() || command.name.len() > 120 {
            return Err(StorageError::InvalidWorkspace);
        }
        let injection_refs = WorkspaceInjectionRefs {
            organization: command.organization_injection_refs.clone(),
            user: command.user_injection_refs.clone(),
        };
        injection_refs.validate()?;
        let creation = creation::WorkspaceCreation {
            command: &command,
            injection_refs: &injection_refs,
            inline,
            admitted_template_yaml,
            allow_cluster_access,
            actor_user_id,
            now,
        };
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                let workspace =
                    creation::create_sqlite(&mut transaction, installation_id.as_str(), &creation)
                        .await?;
                transaction.commit().await?;
                Ok(workspace)
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                let workspace = creation::create_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    &creation,
                )
                .await?;
                transaction.commit().await?;
                Ok(workspace)
            }
        }
    }

    pub async fn get_workspace(&self, workspace_id: Uuid) -> Result<Workspace, StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(&select_workspace_sql("?1", "?2"))
                .bind(installation_id.as_str())
                .bind(workspace_id.to_string())
                .fetch_optional(pool)
                .await?
                .map(decode_sqlite)
                .transpose()?
                .ok_or(StorageError::WorkspaceNotFound),
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(&select_workspace_sql("$1", "$2"))
                .bind(installation_id.as_str())
                .bind(workspace_id.to_string())
                .fetch_optional(pool)
                .await?
                .map(decode_postgres)
                .transpose()?
                .ok_or(StorageError::WorkspaceNotFound),
        }
    }

    pub async fn list_workspaces(
        &self,
        organization_id: Uuid,
    ) -> Result<Vec<Workspace>, StorageError> {
        let sql = format!(
            "SELECT {} FROM workspaces WHERE installation_id = {{install}} AND organization_id = {{organization}} AND state <> 'deleted' ORDER BY created_at, id",
            row::WORKSPACE_COLUMNS
        );
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(
                &sql.replace("{install}", "?1")
                    .replace("{organization}", "?2"),
            )
            .bind(installation_id.as_str())
            .bind(organization_id.to_string())
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(decode_sqlite)
            .collect(),
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(
                &sql.replace("{install}", "$1")
                    .replace("{organization}", "$2"),
            )
            .bind(installation_id.as_str())
            .bind(organization_id.to_string())
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(decode_postgres)
            .collect(),
        }
    }

    pub async fn list_workspaces_page(
        &self,
        organization_id: Uuid,
        limit: Option<u32>,
        cursor: Option<&str>,
        search: Option<&str>,
    ) -> Result<WorkspacePage, StorageError> {
        let limit = i64::from(limit.unwrap_or(50).clamp(1, 200));
        let cursor = cursor
            .map(|value| {
                let bytes = URL_SAFE_NO_PAD
                    .decode(value)
                    .map_err(|_| StorageError::InvalidWorkspace)?;
                serde_json::from_slice::<WorkspaceCursor>(&bytes)
                    .map_err(|_| StorageError::InvalidWorkspace)
            })
            .transpose()?;
        let search = search.unwrap_or("").trim();
        let pattern = format!("%{search}%");
        let sql = format!(
            "SELECT {} FROM workspaces WHERE installation_id = {{install}} AND organization_id = {{organization}} AND state <> 'deleted' \
             AND ({{search}} = '' OR name {{like}} {{pattern}}) \
             AND ({{cursor_created}} IS NULL OR created_at > {{cursor_created}} OR (created_at = {{cursor_created}} AND id > {{cursor_id}})) \
             ORDER BY created_at, id LIMIT {{limit}}",
            row::WORKSPACE_COLUMNS
        );
        let rows = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(
                &sql.replace("{install}", "?1")
                    .replace("{organization}", "?2")
                    .replace("{search}", "?3")
                    .replace("{like}", "LIKE")
                    .replace("{pattern}", "?4")
                    .replace("{cursor_created}", "?5")
                    .replace("{cursor_id}", "?6")
                    .replace("{limit}", "?7"),
            )
            .bind(installation_id.as_str())
            .bind(organization_id.to_string())
            .bind(search)
            .bind(&pattern)
            .bind(cursor.as_ref().map(|c| c.created_at))
            .bind(cursor.as_ref().map(|c| c.id.to_string()))
            .bind(limit + 1)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(decode_sqlite)
            .collect::<Result<Vec<_>, _>>()?,
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(
                &sql.replace("{install}", "$1")
                    .replace("{organization}", "$2")
                    .replace("{search}", "$3")
                    .replace("{like}", "ILIKE")
                    .replace("{pattern}", "$4")
                    .replace("{cursor_created}", "$5")
                    .replace("{cursor_id}", "$6")
                    .replace("{limit}", "$7"),
            )
            .bind(installation_id.as_str())
            .bind(organization_id.to_string())
            .bind(search)
            .bind(&pattern)
            .bind(cursor.as_ref().map(|c| c.created_at))
            .bind(cursor.as_ref().map(|c| c.id.to_string()))
            .bind(limit + 1)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(decode_postgres)
            .collect::<Result<Vec<_>, _>>()?,
        };
        let mut items = rows;
        let next_cursor = if items.len() > limit as usize {
            items.pop();
            let tail = items.last().expect("page has an item");
            Some(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&WorkspaceCursor {
                created_at: tail.created_at,
                id: tail.id,
            })?))
        } else {
            None
        };
        Ok(WorkspacePage { items, next_cursor })
    }
}
