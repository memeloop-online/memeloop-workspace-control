use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

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

pub use pagination::WorkspaceSort;
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
    /// Optional temporary storage capacity in GiB (1–2048). Omitted or null uses the template
    /// default; the chosen capacity is copied into the immutable workspace snapshot.
    #[serde(default)]
    #[schema(minimum = 1, maximum = 2048)]
    pub temporary_storage_gib: Option<u64>,
    /// Store the persistent Home on Longhorn with two live replicas.
    /// Defaults to local storage; temporary storage is always local.
    #[serde(default)]
    pub use_longhorn: bool,
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
    pub node_pool: Option<&'a str>,
    pub inline_injections: Option<(&'a EnvelopeCipher, &'a [InjectionItem])>,
    pub admitted_template_yaml: &'a str,
    /// Final ratio approved before plugin admission; storage must not re-read global settings.
    pub cpu_overcommit_ratio: u16,
    pub allow_cluster_access: bool,
    pub actor_user_id: Uuid,
    pub now: i64,
}

struct WorkspaceCreationOptions<'a> {
    node_pool: Option<&'a str>,
    inline: Option<(&'a EnvelopeCipher, &'a [InjectionItem])>,
    admitted_template_yaml: Option<&'a str>,
    cpu_overcommit_ratio: Option<u16>,
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
    pub requested: crate::quota::QuotaResources,
    pub temporary_requested_gib: u64,
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
                node_pool: None,
                inline: None,
                admitted_template_yaml: None,
                cpu_overcommit_ratio: None,
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
                node_pool: None,
                inline: Some((cipher, inline)),
                admitted_template_yaml: None,
                cpu_overcommit_ratio: None,
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
            node_pool,
            inline_injections,
            admitted_template_yaml,
            cpu_overcommit_ratio,
            allow_cluster_access,
            actor_user_id,
            now,
        } = creation;
        self.create_workspace_inner(
            command,
            WorkspaceCreationOptions {
                node_pool,
                inline: inline_injections,
                admitted_template_yaml: Some(admitted_template_yaml),
                cpu_overcommit_ratio: Some(cpu_overcommit_ratio),
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
        let cpu_overcommit_ratio = match options.cpu_overcommit_ratio {
            Some(ratio) => ratio,
            None => self.cpu_overcommit_ratio().await?,
        };
        if !(1..=1_000).contains(&cpu_overcommit_ratio) {
            return Err(StorageError::InvalidWorkspace);
        }
        let creation = creation::WorkspaceCreation {
            command: &command,
            requested_node_pool: options.node_pool,
            injection_refs: &injection_refs,
            inline: options.inline,
            admitted_template_yaml: options.admitted_template_yaml,
            cpu_overcommit_ratio,
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
        let started = Instant::now();
        let (mut acquire_ms, mut query_ms, mut decode_ms) = (0.0, 0.0, 0.0);
        let result = async {
            match self {
                Self::Sqlite {
                    pool,
                    installation_id,
                } => {
                    let acquire_started = Instant::now();
                    let acquired = pool.acquire().await;
                    acquire_ms = acquire_started.elapsed().as_secs_f64() * 1000.0;
                    let mut connection = acquired?;
                    let query_started = Instant::now();
                    let fetched = sqlx::query(&select_workspace_sql("?1", "?2"))
                        .bind(installation_id.as_str())
                        .bind(workspace_id.to_string())
                        .fetch_optional(&mut *connection)
                        .await;
                    query_ms = query_started.elapsed().as_secs_f64() * 1000.0;
                    drop(connection);
                    let row = fetched?;
                    let decode_started = Instant::now();
                    let decoded = row
                        .map(|row| decode_sqlite(row, installation_id))
                        .transpose()
                        .and_then(|workspace| workspace.ok_or(StorageError::WorkspaceNotFound));
                    decode_ms = decode_started.elapsed().as_secs_f64() * 1000.0;
                    decoded
                }
                Self::Postgres {
                    pool,
                    installation_id,
                } => {
                    let acquire_started = Instant::now();
                    let acquired = pool.acquire().await;
                    acquire_ms = acquire_started.elapsed().as_secs_f64() * 1000.0;
                    let mut connection = acquired?;
                    let query_started = Instant::now();
                    let fetched = sqlx::query(&select_workspace_sql("$1", "$2"))
                        .bind(installation_id.as_str())
                        .bind(workspace_id.to_string())
                        .fetch_optional(&mut *connection)
                        .await;
                    query_ms = query_started.elapsed().as_secs_f64() * 1000.0;
                    drop(connection);
                    let row = fetched?;
                    let decode_started = Instant::now();
                    let decoded = row
                        .map(|row| decode_postgres(row, installation_id))
                        .transpose()
                        .and_then(|workspace| workspace.ok_or(StorageError::WorkspaceNotFound));
                    decode_ms = decode_started.elapsed().as_secs_f64() * 1000.0;
                    decoded
                }
            }
        }
        .await;
        if started.elapsed() >= Duration::from_secs(1) || result.is_err() {
            tracing::info!(
                operation = "get_workspace",
                scope_kind = "workspace",
                acquire_ms,
                query_ms,
                decode_ms,
                success = result.is_ok(),
                "workspace response storage read",
            );
        }
        result
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

#[cfg(test)]
mod tests {
    use super::CreateWorkspace;

    #[test]
    fn omitted_longhorn_option_uses_local_home() {
        let command: CreateWorkspace = serde_json::from_value(serde_json::json!({
            "organization_id": "00000000-0000-7000-8000-000000000001",
            "owner_id": "00000000-0000-7000-8000-000000000002",
            "template_id": "00000000-0000-7000-8000-000000000003",
            "name": "local-home"
        }))
        .unwrap();
        assert!(!command.use_longhorn);
    }
}
