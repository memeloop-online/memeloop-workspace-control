use sqlx::{PgConnection, SqliteConnection};
use uuid::Uuid;

use crate::{
    workspace_runtime::{WorkspaceRuntimeIdentity, WorkspaceRuntimeNamingScheme},
    workspaces::{Workspace, WorkspaceState},
};

use super::{
    Database, StorageError,
    workspace_store::{decode_postgres, decode_sqlite, select_workspace_sql},
};

impl Database {
    /// Atomically switches one stopped workspace to the collision-free runtime identity.
    ///
    /// Operators must prepare and verify the target PVC, remove the old StatefulSet, and prove
    /// that no workspace Pod exists before invoking this transition. The database additionally
    /// rejects active reconcile jobs so an old-identity worker cannot race the cutover.
    pub async fn canonicalize_workspace_runtime(
        &self,
        workspace_id: Uuid,
        now: i64,
    ) -> Result<Workspace, StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                let mut workspace = sqlx::query(&select_workspace_sql("?1", "?2"))
                    .bind(installation_id.as_str())
                    .bind(workspace_id.to_string())
                    .fetch_optional(&mut *transaction)
                    .await?
                    .map(|row| decode_sqlite(row, installation_id))
                    .transpose()?
                    .ok_or(StorageError::WorkspaceNotFound)?;
                canonicalize_sqlite(&mut transaction, installation_id, &mut workspace, now).await?;
                transaction.commit().await?;
                Ok(workspace)
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                let mut workspace =
                    sqlx::query(&format!("{} FOR UPDATE", select_workspace_sql("$1", "$2")))
                        .bind(installation_id.as_str())
                        .bind(workspace_id.to_string())
                        .fetch_optional(&mut *transaction)
                        .await?
                        .map(|row| decode_postgres(row, installation_id))
                        .transpose()?
                        .ok_or(StorageError::WorkspaceNotFound)?;
                canonicalize_postgres(&mut transaction, installation_id, &mut workspace, now)
                    .await?;
                transaction.commit().await?;
                Ok(workspace)
            }
        }
    }
}

fn canonical_identity(
    installation_id: &crate::config::InstallationId,
    workspace: &Workspace,
) -> Result<Option<WorkspaceRuntimeIdentity>, StorageError> {
    if workspace.state != WorkspaceState::Stopped {
        return Err(StorageError::WorkspaceRuntimeMigrationUnsafe);
    }
    if workspace.runtime.naming_scheme == WorkspaceRuntimeNamingScheme::PrefixedV2 {
        return Ok(None);
    }
    let identity = WorkspaceRuntimeIdentity::prefixed_v2(
        installation_id,
        workspace.id,
        &workspace.short_id,
        None,
    )
    .map_err(|_| StorageError::InvalidWorkspace)?;
    if identity.namespace != workspace.runtime.namespace {
        return Err(StorageError::WorkspaceRuntimeMigrationUnsafe);
    }
    Ok(Some(identity))
}

async fn canonicalize_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &crate::config::InstallationId,
    workspace: &mut Workspace,
    now: i64,
) -> Result<(), StorageError> {
    let Some(identity) = canonical_identity(installation_id, workspace)? else {
        return Ok(());
    };
    ensure_no_active_jobs_sqlite(connection, installation_id.as_str(), workspace.id).await?;
    let generation = workspace
        .generation
        .checked_add(1)
        .ok_or(StorageError::InvalidWorkspace)?;
    let affected = sqlx::query(
        "UPDATE workspaces SET runtime_naming_scheme = ?1, runtime_namespace_scope = ?2, \
         runtime_namespace = ?3, runtime_resource_prefix = ?4, runtime_route_key = ?5, \
         generation = ?6, updated_at = ?7 WHERE installation_id = ?8 AND id = ?9 \
         AND state = 'stopped' AND runtime_naming_scheme = 'legacy_v1' AND generation = ?10",
    )
    .bind(identity.naming_scheme.as_str())
    .bind(identity.namespace_scope.as_str())
    .bind(&identity.namespace)
    .bind(&identity.resource_prefix)
    .bind(&identity.route_key)
    .bind(i64::try_from(generation).map_err(|_| StorageError::InvalidWorkspace)?)
    .bind(now)
    .bind(installation_id.as_str())
    .bind(workspace.id.to_string())
    .bind(i64::try_from(workspace.generation).map_err(|_| StorageError::InvalidWorkspace)?)
    .execute(&mut *connection)
    .await?
    .rows_affected();
    if affected != 1 {
        return Err(StorageError::WorkspaceRuntimeMigrationUnsafe);
    }
    workspace.runtime = identity;
    workspace.generation = generation;
    enqueue_and_record_sqlite(connection, installation_id.as_str(), workspace, now).await
}

async fn canonicalize_postgres(
    connection: &mut PgConnection,
    installation_id: &crate::config::InstallationId,
    workspace: &mut Workspace,
    now: i64,
) -> Result<(), StorageError> {
    let Some(identity) = canonical_identity(installation_id, workspace)? else {
        return Ok(());
    };
    ensure_no_active_jobs_postgres(connection, installation_id.as_str(), workspace.id).await?;
    let generation = workspace
        .generation
        .checked_add(1)
        .ok_or(StorageError::InvalidWorkspace)?;
    let affected = sqlx::query(
        "UPDATE workspaces SET runtime_naming_scheme = $1, runtime_namespace_scope = $2, \
         runtime_namespace = $3, runtime_resource_prefix = $4, runtime_route_key = $5, \
         generation = $6, updated_at = $7 WHERE installation_id = $8 AND id = $9 \
         AND state = 'stopped' AND runtime_naming_scheme = 'legacy_v1' AND generation = $10",
    )
    .bind(identity.naming_scheme.as_str())
    .bind(identity.namespace_scope.as_str())
    .bind(&identity.namespace)
    .bind(&identity.resource_prefix)
    .bind(&identity.route_key)
    .bind(i64::try_from(generation).map_err(|_| StorageError::InvalidWorkspace)?)
    .bind(now)
    .bind(installation_id.as_str())
    .bind(workspace.id.to_string())
    .bind(i64::try_from(workspace.generation).map_err(|_| StorageError::InvalidWorkspace)?)
    .execute(&mut *connection)
    .await?
    .rows_affected();
    if affected != 1 {
        return Err(StorageError::WorkspaceRuntimeMigrationUnsafe);
    }
    workspace.runtime = identity;
    workspace.generation = generation;
    enqueue_and_record_postgres(connection, installation_id.as_str(), workspace, now).await
}

async fn ensure_no_active_jobs_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace_id: Uuid,
) -> Result<(), StorageError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM jobs WHERE installation_id = ?1 AND workspace_id = ?2 \
         AND status IN ('pending', 'running')",
    )
    .bind(installation_id)
    .bind(workspace_id.to_string())
    .fetch_one(connection)
    .await?;
    if count != 0 {
        return Err(StorageError::WorkspaceRuntimeMigrationUnsafe);
    }
    Ok(())
}

async fn ensure_no_active_jobs_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace_id: Uuid,
) -> Result<(), StorageError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM jobs WHERE installation_id = $1 AND workspace_id = $2 \
         AND status IN ('pending', 'running')",
    )
    .bind(installation_id)
    .bind(workspace_id.to_string())
    .fetch_one(connection)
    .await?;
    if count != 0 {
        return Err(StorageError::WorkspaceRuntimeMigrationUnsafe);
    }
    Ok(())
}

async fn enqueue_and_record_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace: &Workspace,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES (?1, ?2, 'reconcile_workspace', ?3, ?4, 'pending', ?5, NULL, NULL, 0, ?5, ?5)")
        .bind(Uuid::now_v7().to_string()).bind(installation_id).bind(workspace.id.to_string())
        .bind(serde_json::json!({"generation": workspace.generation, "action": "canonicalize_runtime"}).to_string())
        .bind(now).execute(&mut *connection).await?;
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES (?1, ?2, NULL, ?3, ?4, 'workspace.runtime_canonicalized', '{}', ?5)")
        .bind(Uuid::now_v7().to_string()).bind(installation_id)
        .bind(workspace.organization_id.to_string()).bind(workspace.id.to_string()).bind(now)
        .execute(&mut *connection).await?;
    super::workspace_events::insert_sqlite(
        connection,
        installation_id,
        workspace,
        Some("runtime_canonicalized"),
        now,
    )
    .await
}

async fn enqueue_and_record_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &Workspace,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES ($1, $2, 'reconcile_workspace', $3, $4, 'pending', $5, NULL, NULL, 0, $5, $5)")
        .bind(Uuid::now_v7().to_string()).bind(installation_id).bind(workspace.id.to_string())
        .bind(serde_json::json!({"generation": workspace.generation, "action": "canonicalize_runtime"}).to_string())
        .bind(now).execute(&mut *connection).await?;
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES ($1, $2, NULL, $3, $4, 'workspace.runtime_canonicalized', '{}', $5)")
        .bind(Uuid::now_v7().to_string()).bind(installation_id)
        .bind(workspace.organization_id.to_string()).bind(workspace.id.to_string()).bind(now)
        .execute(&mut *connection).await?;
    super::workspace_events::insert_postgres(
        connection,
        installation_id,
        workspace,
        Some("runtime_canonicalized"),
        now,
    )
    .await
}
