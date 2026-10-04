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
    pub async fn correct_stopped_workspace_home_capacity(
        &self,
        workspace_id: Uuid,
        disk_gib: u64,
        expected_generation: u64,
        actor_user_id: Uuid,
        now: i64,
        claim_uid: &str,
    ) -> Result<Workspace, StorageError> {
        if disk_gib == 0
            || disk_gib
                .checked_mul(1 << 30)
                .is_none_or(|bytes| bytes > i64::MAX as u64)
        {
            return Err(StorageError::InvalidWorkspaceHomePvc);
        }
        let update = HomeCapacityCorrection {
            disk_gib,
            claim_uid,
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
struct HomeCapacityCorrection<'a> {
    claim_uid: &'a str,
    disk_gib: u64,
    expected_generation: u64,
    actor_user_id: Uuid,
    now: i64,
}

async fn update_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace: &mut Workspace,
    snapshot_yaml: &str,
    update: HomeCapacityCorrection<'_>,
) -> Result<(), StorageError> {
    ensure_stopped_at_generation(workspace, update.expected_generation)?;
    let previous_storage_gib = workspace.template.resources.disk_gib;
    let snapshot_yaml = apply_resize(workspace, snapshot_yaml, update.disk_gib, update.now)?;
    let affected = sqlx::query("UPDATE workspaces SET disk_gib = ?1, template_snapshot_yaml = ?2, generation = ?3, updated_at = ?4 WHERE installation_id = ?5 AND id = ?6 AND state = 'stopped' AND home_pvc_name IS NULL AND generation = ?7")
        .bind(as_i64(update.disk_gib)?)
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
        return Err(StorageError::WorkspaceHomePvcUpdateConflict);
    }
    record_side_effects_sqlite(
        connection,
        installation_id,
        workspace,
        previous_storage_gib,
        update,
    )
    .await
}

async fn update_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &mut Workspace,
    snapshot_yaml: &str,
    update: HomeCapacityCorrection<'_>,
) -> Result<(), StorageError> {
    ensure_stopped_at_generation(workspace, update.expected_generation)?;
    let previous_storage_gib = workspace.template.resources.disk_gib;
    let snapshot_yaml = apply_resize(workspace, snapshot_yaml, update.disk_gib, update.now)?;
    let affected = sqlx::query("UPDATE workspaces SET disk_gib = $1, template_snapshot_yaml = $2, generation = $3, updated_at = $4 WHERE installation_id = $5 AND id = $6 AND state = 'stopped' AND home_pvc_name IS NULL AND generation = $7")
        .bind(as_i64(update.disk_gib)?)
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
        return Err(StorageError::WorkspaceHomePvcUpdateConflict);
    }
    record_side_effects_postgres(
        connection,
        installation_id,
        workspace,
        previous_storage_gib,
        update,
    )
    .await
}

fn ensure_stopped_at_generation(
    workspace: &Workspace,
    expected_generation: u64,
) -> Result<(), StorageError> {
    (workspace.state == WorkspaceState::Stopped
        && workspace.home_volume_binding.is_none()
        && workspace.generation == expected_generation)
        .then_some(())
        .ok_or(StorageError::WorkspaceHomePvcUpdateConflict)
}

fn apply_resize(
    workspace: &mut Workspace,
    snapshot_yaml: &str,
    disk_gib: u64,
    now: i64,
) -> Result<String, StorageError> {
    let mut snapshot = WorkspaceTemplateDocument::parse(snapshot_yaml)
        .map_err(|_| StorageError::InvalidWorkspace)?;
    snapshot.spec.resources.disk_gib = disk_gib;
    workspace.template.resources.disk_gib = disk_gib;
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
    previous_storage_gib: u64,
    update: HomeCapacityCorrection<'_>,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES (?1, ?2, 'reconcile_workspace', ?3, ?4, 'pending', ?5, NULL, NULL, 0, ?5, ?5)")
        .bind(Uuid::now_v7().to_string())
        .bind(installation_id)
        .bind(workspace.id.to_string())
        .bind(serde_json::json!({"generation": workspace.generation, "reason": "home_capacity_corrected"}).to_string())
        .bind(update.now)
        .execute(&mut *connection)
        .await?;
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, 'workspace.home_capacity_corrected', ?6, ?7)")
        .bind(Uuid::now_v7().to_string())
        .bind(installation_id)
        .bind(update.actor_user_id.to_string())
        .bind(workspace.organization_id.to_string())
        .bind(workspace.id.to_string())
        .bind(serde_json::json!({"claim_uid": update.claim_uid, "claim_name": format!("workspace-data-w-{}-0", workspace.short_id), "previous_disk_gib": previous_storage_gib, "disk_gib": workspace.template.resources.disk_gib, "generation": workspace.generation}).to_string())
        .bind(update.now)
        .execute(&mut *connection)
        .await?;
    Ok(())
}

async fn record_side_effects_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &Workspace,
    previous_storage_gib: u64,
    update: HomeCapacityCorrection<'_>,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES ($1, $2, 'reconcile_workspace', $3, $4, 'pending', $5, NULL, NULL, 0, $5, $5)")
        .bind(Uuid::now_v7().to_string())
        .bind(installation_id)
        .bind(workspace.id.to_string())
        .bind(serde_json::json!({"generation": workspace.generation, "reason": "home_capacity_corrected"}).to_string())
        .bind(update.now)
        .execute(&mut *connection)
        .await?;
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES ($1, $2, $3, $4, $5, 'workspace.home_capacity_corrected', $6, $7)")
        .bind(Uuid::now_v7().to_string())
        .bind(installation_id)
        .bind(update.actor_user_id.to_string())
        .bind(workspace.organization_id.to_string())
        .bind(workspace.id.to_string())
        .bind(serde_json::json!({"claim_uid": update.claim_uid, "claim_name": format!("workspace-data-w-{}-0", workspace.short_id), "previous_disk_gib": previous_storage_gib, "disk_gib": workspace.template.resources.disk_gib, "generation": workspace.generation}).to_string())
        .bind(update.now)
        .execute(&mut *connection)
        .await?;
    Ok(())
}

fn as_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::InvalidWorkspaceHomePvc)
}
