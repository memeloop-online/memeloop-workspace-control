use sqlx::{PgConnection, Row, SqliteConnection};
use uuid::Uuid;

use super::{
    Database, StorageError,
    workspace_store::{decode_postgres, decode_sqlite, select_workspace_sql},
};
use crate::{
    templates::WorkspaceTemplateDocument,
    workspaces::{Workspace, WorkspaceState},
};

pub struct WorkspaceSettingsUpdate {
    pub name: String,
    pub cpu_millis: u64,
    pub memory_mib: u64,
    pub gpu_count: u32,
    pub expected_generation: u64,
    pub actor_user_id: Uuid,
    pub now: i64,
}

impl Database {
    pub async fn update_workspace_settings(
        &self,
        workspace_id: Uuid,
        update: WorkspaceSettingsUpdate,
    ) -> Result<Workspace, StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut tx = pool.begin().await?;
                let row = sqlx::query(&select_workspace_sql("?1", "?2"))
                    .bind(installation_id.as_str())
                    .bind(workspace_id.to_string())
                    .fetch_optional(&mut *tx)
                    .await?
                    .ok_or(StorageError::WorkspaceNotFound)?;
                let snapshot_yaml: String = row.try_get("template_snapshot_yaml")?;
                let mut workspace = decode_sqlite(row, installation_id)?;
                super::workspace_storage_shrink::guard_sqlite(
                    &mut tx,
                    installation_id.as_str(),
                    workspace_id,
                )
                .await?;
                let old_state = workspace.state.as_str().to_owned();
                let (yaml, compute_changed) = apply(&mut workspace, &snapshot_yaml, &update)?;
                if compute_changed {
                    admit_sqlite(&mut tx, installation_id.as_str(), &workspace).await?;
                }
                let affected = sqlx::query("UPDATE workspaces SET name = ?1, cpu_millis = ?2, memory_mib = ?3, gpu_count = ?4, template_snapshot_yaml = ?5, generation = ?6, updated_at = ?7 WHERE installation_id = ?8 AND id = ?9 AND generation = ?10 AND state = ?11")
                    .bind(&workspace.name).bind(as_i64(update.cpu_millis)?).bind(as_i64(update.memory_mib)?)
                    .bind(i64::from(update.gpu_count)).bind(yaml).bind(as_i64(workspace.generation)?).bind(update.now)
                    .bind(installation_id.as_str()).bind(workspace_id.to_string()).bind(as_i64(update.expected_generation)?)
                    .bind(old_state).execute(&mut *tx).await?.rows_affected();
                if affected != 1 {
                    return Err(StorageError::WorkspaceSettingsUpdateConflict);
                }
                if compute_changed {
                    sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES (?1, ?2, 'reconcile_workspace', ?3, ?4, 'pending', ?5, NULL, NULL, 0, ?5, ?5)")
                        .bind(Uuid::now_v7().to_string()).bind(installation_id.as_str()).bind(workspace_id.to_string())
                        .bind(serde_json::json!({"generation":workspace.generation,"reason":"settings_updated"}).to_string())
                        .bind(update.now).execute(&mut *tx).await?;
                }
                sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, 'workspace.settings_updated', ?6, ?7)")
                    .bind(Uuid::now_v7().to_string()).bind(installation_id.as_str()).bind(update.actor_user_id.to_string())
                    .bind(workspace.organization_id.to_string()).bind(workspace_id.to_string())
                    .bind(serde_json::json!({"name":workspace.name,"resources":workspace.template.resources,"generation":workspace.generation}).to_string())
                    .bind(update.now).execute(&mut *tx).await?;
                super::workspace_events::insert_sqlite(
                    &mut tx,
                    installation_id.as_str(),
                    &workspace,
                    Some("settings_updated"),
                    update.now,
                )
                .await?;
                tx.commit().await?;
                Ok(workspace)
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut tx = pool.begin().await?;
                let row = sqlx::query(&format!("{} FOR UPDATE", select_workspace_sql("$1", "$2")))
                    .bind(installation_id.as_str())
                    .bind(workspace_id.to_string())
                    .fetch_optional(&mut *tx)
                    .await?
                    .ok_or(StorageError::WorkspaceNotFound)?;
                let snapshot_yaml: String = row.try_get("template_snapshot_yaml")?;
                let mut workspace = decode_postgres(row, installation_id)?;
                super::workspace_storage_shrink::guard_postgres(
                    &mut tx,
                    installation_id.as_str(),
                    workspace_id,
                )
                .await?;
                let old_state = workspace.state.as_str().to_owned();
                let (yaml, compute_changed) = apply(&mut workspace, &snapshot_yaml, &update)?;
                if compute_changed {
                    admit_postgres(&mut tx, installation_id.as_str(), &workspace).await?;
                }
                let affected = sqlx::query("UPDATE workspaces SET name = $1, cpu_millis = $2, memory_mib = $3, gpu_count = $4, template_snapshot_yaml = $5, generation = $6, updated_at = $7 WHERE installation_id = $8 AND id = $9 AND generation = $10 AND state = $11")
                    .bind(&workspace.name).bind(as_i64(update.cpu_millis)?).bind(as_i64(update.memory_mib)?)
                    .bind(i64::from(update.gpu_count)).bind(yaml).bind(as_i64(workspace.generation)?).bind(update.now)
                    .bind(installation_id.as_str()).bind(workspace_id.to_string()).bind(as_i64(update.expected_generation)?)
                    .bind(old_state).execute(&mut *tx).await?.rows_affected();
                if affected != 1 {
                    return Err(StorageError::WorkspaceSettingsUpdateConflict);
                }
                if compute_changed {
                    sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES ($1, $2, 'reconcile_workspace', $3, $4, 'pending', $5, NULL, NULL, 0, $5, $5)")
                        .bind(Uuid::now_v7().to_string()).bind(installation_id.as_str()).bind(workspace_id.to_string())
                        .bind(serde_json::json!({"generation":workspace.generation,"reason":"settings_updated"}).to_string())
                        .bind(update.now).execute(&mut *tx).await?;
                }
                sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES ($1, $2, $3, $4, $5, 'workspace.settings_updated', $6, $7)")
                    .bind(Uuid::now_v7().to_string()).bind(installation_id.as_str()).bind(update.actor_user_id.to_string())
                    .bind(workspace.organization_id.to_string()).bind(workspace_id.to_string())
                    .bind(serde_json::json!({"name":workspace.name,"resources":workspace.template.resources,"generation":workspace.generation}).to_string())
                    .bind(update.now).execute(&mut *tx).await?;
                super::workspace_events::insert_postgres(
                    &mut tx,
                    installation_id.as_str(),
                    &workspace,
                    Some("settings_updated"),
                    update.now,
                )
                .await?;
                tx.commit().await?;
                Ok(workspace)
            }
        }
    }
}

fn apply(
    workspace: &mut Workspace,
    snapshot_yaml: &str,
    update: &WorkspaceSettingsUpdate,
) -> Result<(String, bool), StorageError> {
    if workspace.generation != update.expected_generation
        || !matches!(
            workspace.state,
            WorkspaceState::Ready | WorkspaceState::Stopped
        )
    {
        return Err(StorageError::WorkspaceSettingsUpdateConflict);
    }
    let name = update.name.trim();
    if name.is_empty() || name.chars().count() > 120 || name.chars().any(char::is_control) {
        return Err(StorageError::InvalidWorkspace);
    }
    let mut resources = workspace.template.resources;
    resources.cpu_millis = update.cpu_millis;
    resources.memory_mib = update.memory_mib;
    resources.gpu_count = update.gpu_count;
    let compute_changed = resources != workspace.template.resources;
    if compute_changed && workspace.state != WorkspaceState::Stopped {
        return Err(StorageError::WorkspaceSettingsUpdateConflict);
    }
    let mut snapshot = WorkspaceTemplateDocument::parse(snapshot_yaml)
        .map_err(|_| StorageError::InvalidWorkspace)?;
    snapshot.spec.resources = resources;
    snapshot
        .validate()
        .map_err(|_| StorageError::InvalidWorkspace)?;
    workspace.template = snapshot.spec.clone();
    workspace.name = name.to_owned();
    if compute_changed {
        workspace.generation = workspace
            .generation
            .checked_add(1)
            .ok_or(StorageError::InvalidWorkspace)?;
    }
    workspace.updated_at = update.now;
    Ok((
        snapshot
            .to_yaml()
            .map_err(|_| StorageError::InvalidWorkspace)?,
        compute_changed,
    ))
}

fn as_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::InvalidWorkspace)
}

async fn admit_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace: &Workspace,
) -> Result<(), StorageError> {
    // Reuse creation admission, excluding this workspace's previous allocation.
    for (table, column, owner) in [
        (
            "organization_quotas",
            "organization_id",
            workspace.organization_id,
        ),
        ("user_quotas", "user_id", workspace.owner_id),
    ] {
        let quota = sqlx::query(&format!("SELECT cpu_millis, memory_mib, gpu_count, disk_gib FROM {table} WHERE installation_id = ?1 AND {column} = ?2"))
            .bind(installation_id).bind(owner.to_string()).fetch_optional(&mut *connection).await?;
        if let Some(quota) = quota {
            let usage_column = if column == "user_id" {
                "owner_id"
            } else {
                column
            };
            let usage = sqlx::query(&format!("SELECT COALESCE(SUM(cpu_millis), 0) cpu_millis, COALESCE(SUM(memory_mib), 0) memory_mib, COALESCE(SUM(gpu_count), 0) gpu_count, COALESCE(SUM(disk_gib), 0) disk_gib FROM workspaces WHERE installation_id = ?1 AND {usage_column} = ?2 AND id <> ?3 AND state <> 'deleted'"))
                .bind(installation_id).bind(owner.to_string()).bind(workspace.id.to_string()).fetch_one(&mut *connection).await?;
            super::workspace_admission::admit_rows(
                &quota,
                &usage,
                crate::quota::QuotaResources::for_workspace(
                    workspace.template.resources,
                    workspace.template.storage_policy.temporary_storage_gib,
                ),
            )?;
        }
    }
    Ok(())
}

async fn admit_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &Workspace,
) -> Result<(), StorageError> {
    // Reuse creation admission, excluding this workspace's previous allocation.
    for (table, column, owner) in [
        (
            "organization_quotas",
            "organization_id",
            workspace.organization_id,
        ),
        ("user_quotas", "user_id", workspace.owner_id),
    ] {
        let quota = sqlx::query(&format!("SELECT cpu_millis, memory_mib, gpu_count, disk_gib FROM {table} WHERE installation_id = $1 AND {column} = $2 FOR UPDATE"))
            .bind(installation_id).bind(owner.to_string()).fetch_optional(&mut *connection).await?;
        if let Some(quota) = quota {
            let usage_column = if column == "user_id" {
                "owner_id"
            } else {
                column
            };
            let usage = sqlx::query(&format!("SELECT CAST(COALESCE(SUM(cpu_millis), 0) AS BIGINT) cpu_millis, CAST(COALESCE(SUM(memory_mib), 0) AS BIGINT) memory_mib, CAST(COALESCE(SUM(gpu_count), 0) AS BIGINT) gpu_count, CAST(COALESCE(SUM(disk_gib), 0) AS BIGINT) disk_gib FROM workspaces WHERE installation_id = $1 AND {usage_column} = $2 AND id <> $3 AND state <> 'deleted'"))
                .bind(installation_id).bind(owner.to_string()).bind(workspace.id.to_string()).fetch_one(&mut *connection).await?;
            super::workspace_admission::admit_rows(
                &quota,
                &usage,
                crate::quota::QuotaResources::for_workspace(
                    workspace.template.resources,
                    workspace.template.storage_policy.temporary_storage_gib,
                ),
            )?;
        }
    }
    Ok(())
}
