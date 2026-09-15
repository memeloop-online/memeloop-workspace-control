use sqlx::{PgConnection, SqliteConnection};
use uuid::Uuid;

use crate::workspaces::{Workspace, WorkspaceState};

use super::{
    Database, StorageError,
    workspace_store::{decode_postgres, decode_sqlite, select_workspace_sql},
};

impl Database {
    pub async fn update_stopped_workspace_placement(
        &self,
        workspace_id: Uuid,
        node_pool: &str,
        expected_generation: u64,
        actor_user_id: Uuid,
        now: i64,
    ) -> Result<Workspace, StorageError> {
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
                let mut workspace = decode_sqlite(row, installation_id)?;
                update_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    &mut workspace,
                    node_pool,
                    expected_generation,
                    actor_user_id,
                    now,
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
                let mut workspace = decode_postgres(row, installation_id)?;
                update_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    &mut workspace,
                    node_pool,
                    expected_generation,
                    actor_user_id,
                    now,
                )
                .await?;
                transaction.commit().await?;
                Ok(workspace)
            }
        }
    }
}

async fn update_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace: &mut Workspace,
    node_pool: &str,
    expected_generation: u64,
    actor_user_id: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    ensure_stopped_at_generation(workspace, expected_generation)?;
    let selected = super::node_pool_store::select_node_pool_sqlite(
        connection,
        installation_id,
        &workspace.template.placement,
        Some(node_pool),
    )
    .await?;
    apply_placement(workspace, selected, now)?;
    let affected = sqlx::query("UPDATE workspaces SET node_pool = ?1, generation = ?2, updated_at = ?3 WHERE installation_id = ?4 AND id = ?5 AND state = 'stopped' AND generation = ?6")
        .bind(&workspace.node_pool)
        .bind(as_i64(workspace.generation)?)
        .bind(now)
        .bind(installation_id)
        .bind(workspace.id.to_string())
        .bind(as_i64(expected_generation)?)
        .execute(&mut *connection)
        .await?
        .rows_affected();
    if affected != 1 {
        return Err(StorageError::WorkspacePlacementUpdateConflict);
    }
    record_side_effects_sqlite(connection, installation_id, workspace, actor_user_id, now).await
}

async fn update_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &mut Workspace,
    node_pool: &str,
    expected_generation: u64,
    actor_user_id: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    ensure_stopped_at_generation(workspace, expected_generation)?;
    let selected = super::node_pool_store::select_node_pool_postgres(
        connection,
        installation_id,
        &workspace.template.placement,
        Some(node_pool),
    )
    .await?;
    apply_placement(workspace, selected, now)?;
    let affected = sqlx::query("UPDATE workspaces SET node_pool = $1, generation = $2, updated_at = $3 WHERE installation_id = $4 AND id = $5 AND state = 'stopped' AND generation = $6")
        .bind(&workspace.node_pool)
        .bind(as_i64(workspace.generation)?)
        .bind(now)
        .bind(installation_id)
        .bind(workspace.id.to_string())
        .bind(as_i64(expected_generation)?)
        .execute(&mut *connection)
        .await?
        .rows_affected();
    if affected != 1 {
        return Err(StorageError::WorkspacePlacementUpdateConflict);
    }
    record_side_effects_postgres(connection, installation_id, workspace, actor_user_id, now).await
}

fn ensure_stopped_at_generation(
    workspace: &Workspace,
    expected_generation: u64,
) -> Result<(), StorageError> {
    (workspace.state == WorkspaceState::Stopped && workspace.generation == expected_generation)
        .then_some(())
        .ok_or(StorageError::WorkspacePlacementUpdateConflict)
}

fn apply_placement(
    workspace: &mut Workspace,
    node_pool: String,
    now: i64,
) -> Result<(), StorageError> {
    workspace.node_pool = node_pool;
    workspace.generation = workspace
        .generation
        .checked_add(1)
        .ok_or(StorageError::InvalidWorkspace)?;
    workspace.updated_at = now;
    Ok(())
}

async fn record_side_effects_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace: &Workspace,
    actor_user_id: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES (?1, ?2, 'reconcile_workspace', ?3, ?4, 'pending', ?5, NULL, NULL, 0, ?5, ?5)")
        .bind(Uuid::now_v7().to_string())
        .bind(installation_id)
        .bind(workspace.id.to_string())
        .bind(serde_json::json!({"generation": workspace.generation, "reason": "placement_updated"}).to_string())
        .bind(now)
        .execute(&mut *connection)
        .await?;
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, 'workspace.placement_updated', ?6, ?7)")
        .bind(Uuid::now_v7().to_string())
        .bind(installation_id)
        .bind(actor_user_id.to_string())
        .bind(workspace.organization_id.to_string())
        .bind(workspace.id.to_string())
        .bind(serde_json::json!({"node_pool": workspace.node_pool, "generation": workspace.generation}).to_string())
        .bind(now)
        .execute(&mut *connection)
        .await?;
    super::workspace_events::insert_placement_updated_sqlite(
        connection,
        installation_id,
        workspace,
        now,
    )
    .await
}

async fn record_side_effects_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &Workspace,
    actor_user_id: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES ($1, $2, 'reconcile_workspace', $3, $4, 'pending', $5, NULL, NULL, 0, $5, $5)")
        .bind(Uuid::now_v7().to_string())
        .bind(installation_id)
        .bind(workspace.id.to_string())
        .bind(serde_json::json!({"generation": workspace.generation, "reason": "placement_updated"}).to_string())
        .bind(now)
        .execute(&mut *connection)
        .await?;
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES ($1, $2, $3, $4, $5, 'workspace.placement_updated', $6, $7)")
        .bind(Uuid::now_v7().to_string())
        .bind(installation_id)
        .bind(actor_user_id.to_string())
        .bind(workspace.organization_id.to_string())
        .bind(workspace.id.to_string())
        .bind(serde_json::json!({"node_pool": workspace.node_pool, "generation": workspace.generation}).to_string())
        .bind(now)
        .execute(&mut *connection)
        .await?;
    super::workspace_events::insert_placement_updated_postgres(
        connection,
        installation_id,
        workspace,
        now,
    )
    .await
}

fn as_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::InvalidWorkspace)
}
