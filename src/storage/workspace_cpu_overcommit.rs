use sqlx::{PgConnection, Row, SqliteConnection};
use uuid::Uuid;

use crate::{
    templates::WorkspaceTemplateDocument,
    workspaces::{Workspace, WorkspaceState},
};

use super::{
    Database, StorageError,
    workspace_store::{decode_postgres, decode_sqlite, select_workspace_sql},
};

impl Database {
    pub async fn update_stopped_workspace_cpu_overcommit(
        &self,
        workspace_id: Uuid,
        cpu_overcommit_ratio: u16,
        expected_generation: u64,
        actor_user_id: Uuid,
        now: i64,
    ) -> Result<Workspace, StorageError> {
        if !(1..=super::MAX_CPU_OVERCOMMIT_RATIO).contains(&cpu_overcommit_ratio) {
            return Err(StorageError::InvalidCpuOvercommitRatio);
        }
        let update = CpuOvercommitUpdate {
            cpu_overcommit_ratio,
            expected_generation,
            actor_user_id,
            now,
        };
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                let row = sqlx::query(&select_workspace_sql("?1", "?2"))
                    .bind(installation_id.as_str())
                    .bind(workspace_id.to_string())
                    .fetch_optional(&mut *transaction)
                    .await?
                    .ok_or(StorageError::WorkspaceNotFound)?;
                let snapshot_yaml: String = row.try_get("template_snapshot_yaml")?;
                let mut workspace = decode_sqlite(row, installation_id)?;
                update_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    &mut workspace,
                    &snapshot_yaml,
                    update,
                )
                .await?;
                transaction.commit().await?;
                Ok(workspace)
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                let row = sqlx::query(&format!("{} FOR UPDATE", select_workspace_sql("$1", "$2")))
                    .bind(installation_id.as_str())
                    .bind(workspace_id.to_string())
                    .fetch_optional(&mut *transaction)
                    .await?
                    .ok_or(StorageError::WorkspaceNotFound)?;
                let snapshot_yaml: String = row.try_get("template_snapshot_yaml")?;
                let mut workspace = decode_postgres(row, installation_id)?;
                update_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    &mut workspace,
                    &snapshot_yaml,
                    update,
                )
                .await?;
                transaction.commit().await?;
                Ok(workspace)
            }
        }
    }
}

#[derive(Clone, Copy)]
struct CpuOvercommitUpdate {
    cpu_overcommit_ratio: u16,
    expected_generation: u64,
    actor_user_id: Uuid,
    now: i64,
}

async fn update_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace: &mut Workspace,
    snapshot_yaml: &str,
    update: CpuOvercommitUpdate,
) -> Result<(), StorageError> {
    super::workspace_storage_shrink::guard_sqlite(connection, installation_id, workspace.id)
        .await?;
    ensure_stopped_at_generation(workspace, update.expected_generation)?;
    let previous_ratio = workspace.template.cpu_overcommit_ratio;
    let snapshot_yaml = apply_ratio(
        workspace,
        snapshot_yaml,
        update.cpu_overcommit_ratio,
        update.now,
    )?;
    let affected = sqlx::query("UPDATE workspaces SET template_snapshot_yaml = ?1, generation = ?2, updated_at = ?3 WHERE installation_id = ?4 AND id = ?5 AND state = 'stopped' AND generation = ?6")
        .bind(snapshot_yaml)
        .bind(as_i64(workspace.generation)?)
        .bind(update.now)
        .bind(installation_id)
        .bind(workspace.id.to_string())
        .bind(as_i64(update.expected_generation)?)
        .execute(&mut *connection)
        .await?
        .rows_affected();
    if affected != 1 {
        return Err(StorageError::WorkspaceCpuOvercommitUpdateConflict);
    }
    record_side_effects_sqlite(
        connection,
        installation_id,
        workspace,
        previous_ratio,
        update,
    )
    .await
}

async fn update_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &mut Workspace,
    snapshot_yaml: &str,
    update: CpuOvercommitUpdate,
) -> Result<(), StorageError> {
    super::workspace_storage_shrink::guard_postgres(connection, installation_id, workspace.id)
        .await?;
    ensure_stopped_at_generation(workspace, update.expected_generation)?;
    let previous_ratio = workspace.template.cpu_overcommit_ratio;
    let snapshot_yaml = apply_ratio(
        workspace,
        snapshot_yaml,
        update.cpu_overcommit_ratio,
        update.now,
    )?;
    let affected = sqlx::query("UPDATE workspaces SET template_snapshot_yaml = $1, generation = $2, updated_at = $3 WHERE installation_id = $4 AND id = $5 AND state = 'stopped' AND generation = $6")
        .bind(snapshot_yaml)
        .bind(as_i64(workspace.generation)?)
        .bind(update.now)
        .bind(installation_id)
        .bind(workspace.id.to_string())
        .bind(as_i64(update.expected_generation)?)
        .execute(&mut *connection)
        .await?
        .rows_affected();
    if affected != 1 {
        return Err(StorageError::WorkspaceCpuOvercommitUpdateConflict);
    }
    record_side_effects_postgres(
        connection,
        installation_id,
        workspace,
        previous_ratio,
        update,
    )
    .await
}

fn ensure_stopped_at_generation(
    workspace: &Workspace,
    expected_generation: u64,
) -> Result<(), StorageError> {
    (workspace.state == WorkspaceState::Stopped && workspace.generation == expected_generation)
        .then_some(())
        .ok_or(StorageError::WorkspaceCpuOvercommitUpdateConflict)
}

fn apply_ratio(
    workspace: &mut Workspace,
    snapshot_yaml: &str,
    cpu_overcommit_ratio: u16,
    now: i64,
) -> Result<String, StorageError> {
    let mut snapshot = WorkspaceTemplateDocument::parse(snapshot_yaml)
        .map_err(|_| StorageError::InvalidWorkspace)?;
    snapshot.spec.cpu_overcommit_ratio = Some(cpu_overcommit_ratio);
    workspace.template.cpu_overcommit_ratio = Some(cpu_overcommit_ratio);
    workspace.generation = workspace
        .generation
        .checked_add(1)
        .ok_or(StorageError::InvalidWorkspace)?;
    workspace.updated_at = now;
    snapshot
        .to_yaml()
        .map_err(|_| StorageError::InvalidWorkspace)
}

async fn record_side_effects_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace: &Workspace,
    previous_ratio: Option<u16>,
    update: CpuOvercommitUpdate,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES (?1, ?2, 'reconcile_workspace', ?3, ?4, 'pending', ?5, NULL, NULL, 0, ?5, ?5)")
        .bind(Uuid::now_v7().to_string())
        .bind(installation_id)
        .bind(workspace.id.to_string())
        .bind(serde_json::json!({"generation": workspace.generation, "reason": "cpu_overcommit_updated"}).to_string())
        .bind(update.now)
        .execute(&mut *connection)
        .await?;
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, 'workspace.cpu_overcommit_updated', ?6, ?7)")
        .bind(Uuid::now_v7().to_string())
        .bind(installation_id)
        .bind(update.actor_user_id.to_string())
        .bind(workspace.organization_id.to_string())
        .bind(workspace.id.to_string())
        .bind(serde_json::json!({"previous_cpu_overcommit_ratio": previous_ratio, "cpu_overcommit_ratio": workspace.template.cpu_overcommit_ratio, "generation": workspace.generation}).to_string())
        .bind(update.now)
        .execute(&mut *connection)
        .await?;
    super::workspace_events::insert_sqlite(
        connection,
        installation_id,
        workspace,
        Some("cpu_overcommit_updated"),
        update.now,
    )
    .await
}

async fn record_side_effects_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &Workspace,
    previous_ratio: Option<u16>,
    update: CpuOvercommitUpdate,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES ($1, $2, 'reconcile_workspace', $3, $4, 'pending', $5, NULL, NULL, 0, $5, $5)")
        .bind(Uuid::now_v7().to_string())
        .bind(installation_id)
        .bind(workspace.id.to_string())
        .bind(serde_json::json!({"generation": workspace.generation, "reason": "cpu_overcommit_updated"}).to_string())
        .bind(update.now)
        .execute(&mut *connection)
        .await?;
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES ($1, $2, $3, $4, $5, 'workspace.cpu_overcommit_updated', $6, $7)")
        .bind(Uuid::now_v7().to_string())
        .bind(installation_id)
        .bind(update.actor_user_id.to_string())
        .bind(workspace.organization_id.to_string())
        .bind(workspace.id.to_string())
        .bind(serde_json::json!({"previous_cpu_overcommit_ratio": previous_ratio, "cpu_overcommit_ratio": workspace.template.cpu_overcommit_ratio, "generation": workspace.generation}).to_string())
        .bind(update.now)
        .execute(&mut *connection)
        .await?;
    super::workspace_events::insert_postgres(
        connection,
        installation_id,
        workspace,
        Some("cpu_overcommit_updated"),
        update.now,
    )
    .await
}

fn as_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::InvalidWorkspace)
}
