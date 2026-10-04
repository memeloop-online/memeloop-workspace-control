use sqlx::{PgConnection, Row, SqliteConnection};
use uuid::Uuid;

use crate::{
    templates::WorkspaceTemplateDocument,
    workspaces::{Workspace, WorkspaceHomeVolumeBinding, WorkspaceState},
};

use super::{
    Database, StorageError,
    workspace_store::{decode_postgres, decode_sqlite, select_workspace_sql},
};

struct BindingUpdate<'a> {
    binding: &'a WorkspaceHomeVolumeBinding,
    expected_generation: u64,
    actor_user_id: Uuid,
    now: i64,
    adopt_ready: bool,
}

impl Database {
    pub async fn bind_workspace_home_volume(
        &self,
        workspace_id: Uuid,
        binding: &WorkspaceHomeVolumeBinding,
        expected_generation: u64,
        actor_user_id: Uuid,
        now: i64,
        adopt_ready: bool,
    ) -> Result<Workspace, StorageError> {
        if !binding.is_valid() {
            return Err(StorageError::InvalidWorkspaceHomePvc);
        }
        let update = BindingUpdate {
            binding,
            expected_generation,
            actor_user_id,
            now,
            adopt_ready,
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
                let snapshot: String = row.try_get("template_snapshot_yaml")?;
                let mut workspace = decode_sqlite(row, installation_id)?;
                update_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    &mut workspace,
                    &snapshot,
                    &update,
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
                let snapshot: String = row.try_get("template_snapshot_yaml")?;
                let mut workspace = decode_postgres(row, installation_id)?;
                update_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    &mut workspace,
                    &snapshot,
                    &update,
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
    installation: &str,
    workspace: &mut Workspace,
    snapshot: &str,
    update: &BindingUpdate<'_>,
) -> Result<(), StorageError> {
    ensure_state(workspace, update)?;
    if update.adopt_ready {
        let active: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE workspace_id = ?1 AND kind = 'reconcile_workspace' AND status IN ('pending', 'running')")
            .bind(workspace.id.to_string()).fetch_one(&mut *connection).await?;
        if active != 0 {
            return Err(StorageError::WorkspaceHomePvcUpdateConflict);
        }
    }
    let conflict: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workspaces WHERE id <> ?1 AND state <> 'deleted' AND (home_pvc_name = ?2 OR home_pvc_uid = ?3 OR 'workspace-data-w-' || short_id || '-0' = ?2)")
        .bind(workspace.id.to_string()).bind(&update.binding.claim_name).bind(&update.binding.claim_uid)
        .fetch_one(&mut *connection).await?;
    if conflict != 0 {
        return Err(StorageError::WorkspaceHomePvcInUse);
    }
    let previous = previous_binding(workspace);
    let snapshot = apply_binding(workspace, snapshot, update)?;
    let affected = sqlx::query("UPDATE workspaces SET home_pvc_namespace = ?1, home_pvc_name = ?2, home_pvc_uid = ?3, home_pvc_capacity_gib = ?4, disk_gib = ?4, template_snapshot_yaml = ?5, generation = ?6, updated_at = ?7 WHERE installation_id = ?8 AND id = ?9 AND state = ?11 AND generation = ?10")
        .bind(&update.binding.namespace).bind(&update.binding.claim_name).bind(&update.binding.claim_uid)
        .bind(as_i64(update.binding.capacity_gib)?).bind(snapshot).bind(as_i64(workspace.generation)?)
        .bind(update.now).bind(installation).bind(workspace.id.to_string()).bind(as_i64(update.expected_generation)?).bind(workspace.state.as_str())
        .execute(&mut *connection).await.map_err(binding_error)?.rows_affected();
    if affected != 1 {
        return Err(StorageError::WorkspaceHomePvcUpdateConflict);
    }
    record_sqlite(connection, installation, workspace, update, previous).await
}

async fn update_postgres(
    connection: &mut PgConnection,
    installation: &str,
    workspace: &mut Workspace,
    snapshot: &str,
    update: &BindingUpdate<'_>,
) -> Result<(), StorageError> {
    ensure_state(workspace, update)?;
    if update.adopt_ready {
        let active: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE workspace_id = $1 AND kind = 'reconcile_workspace' AND status IN ('pending', 'running')")
            .bind(workspace.id.to_string()).fetch_one(&mut *connection).await?;
        if active != 0 {
            return Err(StorageError::WorkspaceHomePvcUpdateConflict);
        }
    }
    let conflict: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workspaces WHERE id <> $1 AND state <> 'deleted' AND (home_pvc_name = $2 OR home_pvc_uid = $3 OR 'workspace-data-w-' || short_id || '-0' = $2)")
        .bind(workspace.id.to_string()).bind(&update.binding.claim_name).bind(&update.binding.claim_uid)
        .fetch_one(&mut *connection).await?;
    if conflict != 0 {
        return Err(StorageError::WorkspaceHomePvcInUse);
    }
    let previous = previous_binding(workspace);
    let snapshot = apply_binding(workspace, snapshot, update)?;
    let affected = sqlx::query("UPDATE workspaces SET home_pvc_namespace = $1, home_pvc_name = $2, home_pvc_uid = $3, home_pvc_capacity_gib = $4, disk_gib = $4, template_snapshot_yaml = $5, generation = $6, updated_at = $7 WHERE installation_id = $8 AND id = $9 AND state = $11 AND generation = $10")
        .bind(&update.binding.namespace).bind(&update.binding.claim_name).bind(&update.binding.claim_uid)
        .bind(as_i64(update.binding.capacity_gib)?).bind(snapshot).bind(as_i64(workspace.generation)?)
        .bind(update.now).bind(installation).bind(workspace.id.to_string()).bind(as_i64(update.expected_generation)?).bind(workspace.state.as_str())
        .execute(&mut *connection).await.map_err(binding_error)?.rows_affected();
    if affected != 1 {
        return Err(StorageError::WorkspaceHomePvcUpdateConflict);
    }
    record_postgres(connection, installation, workspace, update, previous).await
}

fn ensure_state(workspace: &Workspace, update: &BindingUpdate<'_>) -> Result<(), StorageError> {
    let expected_state = if update.adopt_ready {
        WorkspaceState::Ready
    } else {
        WorkspaceState::Stopped
    };
    if workspace.state != expected_state || workspace.generation != update.expected_generation {
        return Err(StorageError::WorkspaceHomePvcUpdateConflict);
    }
    Ok(())
}

fn apply_binding(
    workspace: &mut Workspace,
    snapshot: &str,
    update: &BindingUpdate<'_>,
) -> Result<String, StorageError> {
    let mut document =
        WorkspaceTemplateDocument::parse(snapshot).map_err(|_| StorageError::InvalidWorkspace)?;
    document.spec.resources.disk_gib = update.binding.capacity_gib;
    workspace.template.resources.disk_gib = update.binding.capacity_gib;
    workspace.home_volume_binding = Some(update.binding.clone());
    workspace.generation = workspace
        .generation
        .checked_add(1)
        .ok_or(StorageError::InvalidWorkspace)?;
    workspace.updated_at = update.now;
    document
        .to_yaml()
        .map_err(|_| StorageError::InvalidWorkspace)
}

fn previous_binding(workspace: &Workspace) -> serde_json::Value {
    serde_json::json!({"home_volume_binding": workspace.home_volume_binding, "disk_gib": workspace.template.resources.disk_gib})
}

fn audit_metadata(workspace: &Workspace, previous: serde_json::Value) -> String {
    serde_json::json!({"previous": previous, "home_volume_binding": workspace.home_volume_binding, "disk_gib": workspace.template.resources.disk_gib, "generation": workspace.generation, "adopted_ready": workspace.state == WorkspaceState::Ready}).to_string()
}

async fn record_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    workspace: &Workspace,
    update: &BindingUpdate<'_>,
    previous: serde_json::Value,
) -> Result<(), StorageError> {
    if !update.adopt_ready {
        sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES (?1, ?2, 'reconcile_workspace', ?3, ?4, 'pending', ?5, NULL, NULL, 0, ?5, ?5)")
        .bind(Uuid::now_v7().to_string()).bind(installation).bind(workspace.id.to_string())
        .bind(serde_json::json!({"generation": workspace.generation, "reason": "home_volume_bound"}).to_string()).bind(update.now)
        .execute(&mut *connection).await?;
    }
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, 'workspace.home_volume_bound', ?6, ?7)")
        .bind(Uuid::now_v7().to_string()).bind(installation).bind(update.actor_user_id.to_string())
        .bind(workspace.organization_id.to_string()).bind(workspace.id.to_string())
        .bind(audit_metadata(workspace, previous)).bind(update.now).execute(&mut *connection).await?;
    Ok(())
}

async fn record_postgres(
    connection: &mut PgConnection,
    installation: &str,
    workspace: &Workspace,
    update: &BindingUpdate<'_>,
    previous: serde_json::Value,
) -> Result<(), StorageError> {
    if !update.adopt_ready {
        sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES ($1, $2, 'reconcile_workspace', $3, $4, 'pending', $5, NULL, NULL, 0, $5, $5)")
        .bind(Uuid::now_v7().to_string()).bind(installation).bind(workspace.id.to_string())
        .bind(serde_json::json!({"generation": workspace.generation, "reason": "home_volume_bound"}).to_string()).bind(update.now)
        .execute(&mut *connection).await?;
    }
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES ($1, $2, $3, $4, $5, 'workspace.home_volume_bound', $6, $7)")
        .bind(Uuid::now_v7().to_string()).bind(installation).bind(update.actor_user_id.to_string())
        .bind(workspace.organization_id.to_string()).bind(workspace.id.to_string())
        .bind(audit_metadata(workspace, previous)).bind(update.now).execute(&mut *connection).await?;
    Ok(())
}

fn binding_error(error: sqlx::Error) -> StorageError {
    if error
        .as_database_error()
        .is_some_and(|error| error.is_unique_violation())
    {
        StorageError::WorkspaceHomePvcInUse
    } else {
        StorageError::Database(error)
    }
}

fn as_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::InvalidWorkspaceHomePvc)
}
