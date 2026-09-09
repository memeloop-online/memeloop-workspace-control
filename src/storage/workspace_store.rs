use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    crypto::EnvelopeCipher, injections::InjectionItem, quota::Resources, workspaces::Workspace,
};

use super::{Database, StorageError, WorkspaceInjectionRefs};

mod creation;
mod pagination;
mod row;
mod summary;

pub use summary::WorkspaceUsageSummary;

pub(super) use row::{
    WORKSPACE_COLUMNS, decode_postgres, decode_sqlite, select_workspace_by_route_key_sql,
    select_workspace_by_short_id_sql, select_workspace_sql,
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

/// Inputs that are fixed after API and plugin admission has completed.
///
/// Keeping these values together makes it explicit that the stored workspace
/// must use the template and runtime settings the admission step approved.
pub struct AdmittedWorkspaceCreation<'a> {
    pub command: CreateWorkspace,
    pub inline_injections: Option<(&'a EnvelopeCipher, &'a [InjectionItem])>,
    pub admitted_template_yaml: &'a str,
    pub allow_cluster_access: bool,
    pub actor_user_id: Uuid,
    pub now: i64,
}

struct WorkspaceCreationOptions<'a> {
    inline: Option<(&'a EnvelopeCipher, &'a [InjectionItem])>,
    admitted_template_yaml: Option<&'a str>,
    allow_cluster_access: bool,
    actor_user_id: Uuid,
    now: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WorkspacePage {
    pub items: Vec<Workspace>,
    pub next_cursor: Option<String>,
    /// Aggregate for every non-deleted workspace matching the organization and search query.
    /// This deliberately ignores the pagination cursor.
    pub total_count: u64,
    pub requested: Resources,
    pub state_counts: BTreeMap<String, u64>,
}

impl Database {
    /// Returns the aggregate for the complete visible organization scope without loading
    /// individual workspace records. `None` permits every template; `Some([])` permits none.
    pub async fn workspace_usage_summary(
        &self,
        organization_id: Uuid,
        allowed_template_ids: Option<&[Uuid]>,
    ) -> Result<WorkspaceUsageSummary, StorageError> {
        summary::workspace_usage_summary(self, organization_id, allowed_template_ids).await
    }

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
                    .map(|row| decode_sqlite(row, installation_id))
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
                    .map(|row| decode_postgres(row, installation_id))
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
            WorkspaceCreationOptions {
                inline: None,
                admitted_template_yaml: None,
                allow_cluster_access,
                actor_user_id,
                now,
            },
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
            WorkspaceCreationOptions {
                inline: Some((cipher, inline)),
                admitted_template_yaml: None,
                allow_cluster_access,
                actor_user_id,
                now,
            },
        )
        .await
    }

    pub async fn create_workspace_with_admitted_template(
        &self,
        creation: AdmittedWorkspaceCreation<'_>,
    ) -> Result<Workspace, StorageError> {
        let AdmittedWorkspaceCreation {
            command,
            inline_injections,
            admitted_template_yaml,
            allow_cluster_access,
            actor_user_id,
            now,
        } = creation;
        self.create_workspace_inner(
            command,
            WorkspaceCreationOptions {
                inline: inline_injections,
                admitted_template_yaml: Some(admitted_template_yaml),
                allow_cluster_access,
                actor_user_id,
                now,
            },
        )
        .await
    }

    async fn create_workspace_inner(
        &self,
        command: CreateWorkspace,
        options: WorkspaceCreationOptions<'_>,
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
            inline: options.inline,
            admitted_template_yaml: options.admitted_template_yaml,
            allow_cluster_access: options.allow_cluster_access,
            actor_user_id: options.actor_user_id,
            now: options.now,
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
                .map(|row| decode_sqlite(row, installation_id))
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
                .map(|row| decode_postgres(row, installation_id))
                .transpose()?
                .ok_or(StorageError::WorkspaceNotFound),
        }
    }

    pub async fn get_workspace_by_route_key(
        &self,
        route_key: &str,
    ) -> Result<Workspace, StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(&select_workspace_by_route_key_sql("?1", "?2"))
                .bind(installation_id.as_str())
                .bind(route_key)
                .fetch_optional(pool)
                .await?
                .map(|row| decode_sqlite(row, installation_id))
                .transpose()?
                .ok_or(StorageError::WorkspaceNotFound),
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(&select_workspace_by_route_key_sql("$1", "$2"))
                .bind(installation_id.as_str())
                .bind(route_key)
                .fetch_optional(pool)
                .await?
                .map(|row| decode_postgres(row, installation_id))
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
            .map(|row| decode_sqlite(row, installation_id))
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
            .map(|row| decode_postgres(row, installation_id))
            .collect(),
        }
    }
}
