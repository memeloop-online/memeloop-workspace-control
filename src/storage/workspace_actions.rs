use sqlx::{PgConnection, SqliteConnection};
use uuid::Uuid;

use crate::workspaces::{Workspace, WorkspaceAction, WorkspaceObservation, WorkspaceState};

use super::{
    Database, StorageError,
    workspace_store::{decode_postgres, decode_sqlite, select_workspace_sql},
};

impl Database {
    pub async fn request_workspace_action(
        &self,
        workspace_id: Uuid,
        action: WorkspaceAction,
        actor_user_id: Uuid,
        now: i64,
    ) -> Result<Workspace, StorageError> {
        self.transition_workspace(
            workspace_id,
            StoredTransition::Action(action),
            actor_user_id,
            now,
        )
        .await
    }

    pub async fn record_workspace_observation(
        &self,
        workspace_id: Uuid,
        observation: WorkspaceObservation,
        actor_user_id: Uuid,
        now: i64,
    ) -> Result<Workspace, StorageError> {
        self.transition_workspace(
            workspace_id,
            StoredTransition::Observation(observation),
            actor_user_id,
            now,
        )
        .await
    }

    async fn transition_workspace(
        &self,
        workspace_id: Uuid,
        transition: StoredTransition,
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
                    .await?;
                let mut workspace = row
                    .map(decode_sqlite)
                    .transpose()?
                    .ok_or(StorageError::WorkspaceNotFound)?;
                apply_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    &mut workspace,
                    transition,
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
                    .await?;
                let mut workspace = row
                    .map(decode_postgres)
                    .transpose()?
                    .ok_or(StorageError::WorkspaceNotFound)?;
                apply_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    &mut workspace,
                    transition,
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

#[derive(Clone, Copy)]
enum StoredTransition {
    Action(WorkspaceAction),
    Observation(WorkspaceObservation),
}

impl StoredTransition {
    fn next_state(self, state: WorkspaceState) -> Result<WorkspaceState, StorageError> {
        match self {
            Self::Action(action) => Ok(state.request(action)?),
            Self::Observation(observation) => Ok(state.observe(observation)?),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Action(action) => action.as_str(),
            Self::Observation(observation) => observation.as_str(),
        }
    }

    fn enqueues_reconcile(self) -> bool {
        matches!(self, Self::Action(_))
    }

    fn confirms_deletion(self) -> bool {
        matches!(self, Self::Observation(WorkspaceObservation::Deleted))
    }
}

async fn apply_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace: &mut Workspace,
    transition: StoredTransition,
    actor: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    let prior_state = workspace.state;
    workspace.state = transition.next_state(prior_state)?;
    if transition.enqueues_reconcile() {
        workspace.generation = workspace
            .generation
            .checked_add(1)
            .ok_or(StorageError::InvalidWorkspace)?;
    }
    workspace.updated_at = now;
    let affected = sqlx::query("UPDATE workspaces SET state = ?1, generation = ?2, updated_at = ?3 WHERE installation_id = ?4 AND id = ?5 AND state = ?6")
        .bind(workspace.state.as_str()).bind(as_i64(workspace.generation)?).bind(now)
        .bind(installation_id).bind(workspace.id.to_string()).bind(prior_state.as_str())
        .execute(&mut *connection).await?.rows_affected();
    if affected != 1 {
        return Err(StorageError::WorkspaceNotFound);
    }
    job_and_audit_sqlite(
        connection,
        installation_id,
        workspace,
        transition,
        actor,
        now,
    )
    .await?;
    if transition.confirms_deletion() {
        scrub_deleted_sqlite(connection, installation_id, workspace, now).await?;
    }
    Ok(())
}

async fn apply_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &mut Workspace,
    transition: StoredTransition,
    actor: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    let prior_state = workspace.state;
    workspace.state = transition.next_state(prior_state)?;
    if transition.enqueues_reconcile() {
        workspace.generation = workspace
            .generation
            .checked_add(1)
            .ok_or(StorageError::InvalidWorkspace)?;
    }
    workspace.updated_at = now;
    let affected = sqlx::query("UPDATE workspaces SET state = $1, generation = $2, updated_at = $3 WHERE installation_id = $4 AND id = $5 AND state = $6")
        .bind(workspace.state.as_str()).bind(as_i64(workspace.generation)?).bind(now)
        .bind(installation_id).bind(workspace.id.to_string()).bind(prior_state.as_str())
        .execute(&mut *connection).await?.rows_affected();
    if affected != 1 {
        return Err(StorageError::WorkspaceNotFound);
    }
    job_and_audit_postgres(
        connection,
        installation_id,
        workspace,
        transition,
        actor,
        now,
    )
    .await?;
    if transition.confirms_deletion() {
        scrub_deleted_postgres(connection, installation_id, workspace, now).await?;
    }
    Ok(())
}

async fn scrub_deleted_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    workspace: &Workspace,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO workspace_tombstones (installation_id, workspace_id, organization_id, deleted_at) VALUES (?1, ?2, ?3, ?4)").bind(installation).bind(workspace.id.to_string()).bind(workspace.organization_id.to_string()).bind(now).execute(&mut *connection).await?;
    sqlx::query("DELETE FROM injection_items WHERE installation_id = ?1 AND scope = 'workspace' AND scope_id = ?2").bind(installation).bind(workspace.id.to_string()).execute(&mut *connection).await?;
    sqlx::query(
        "DELETE FROM workspace_ssh_identities WHERE installation_id = ?1 AND workspace_id = ?2",
    )
    .bind(installation)
    .bind(workspace.id.to_string())
    .execute(&mut *connection)
    .await?;
    sqlx::query("DELETE FROM web_shell_tickets WHERE installation_id = ?1 AND workspace_id = ?2")
        .bind(installation)
        .bind(workspace.id.to_string())
        .execute(&mut *connection)
        .await?;
    sqlx::query("UPDATE audit_log SET actor_user_id = NULL, metadata_json = '{}' WHERE installation_id = ?1 AND workspace_id = ?2").bind(installation).bind(workspace.id.to_string()).execute(&mut *connection).await?;
    sqlx::query(
        "DELETE FROM workspaces WHERE installation_id = ?1 AND id = ?2 AND state = 'deleted'",
    )
    .bind(installation)
    .bind(workspace.id.to_string())
    .execute(connection)
    .await?;
    Ok(())
}

async fn scrub_deleted_postgres(
    connection: &mut PgConnection,
    installation: &str,
    workspace: &Workspace,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO workspace_tombstones (installation_id, workspace_id, organization_id, deleted_at) VALUES ($1, $2, $3, $4)").bind(installation).bind(workspace.id.to_string()).bind(workspace.organization_id.to_string()).bind(now).execute(&mut *connection).await?;
    sqlx::query("DELETE FROM injection_items WHERE installation_id = $1 AND scope = 'workspace' AND scope_id = $2").bind(installation).bind(workspace.id.to_string()).execute(&mut *connection).await?;
    sqlx::query(
        "DELETE FROM workspace_ssh_identities WHERE installation_id = $1 AND workspace_id = $2",
    )
    .bind(installation)
    .bind(workspace.id.to_string())
    .execute(&mut *connection)
    .await?;
    sqlx::query("DELETE FROM web_shell_tickets WHERE installation_id = $1 AND workspace_id = $2")
        .bind(installation)
        .bind(workspace.id.to_string())
        .execute(&mut *connection)
        .await?;
    sqlx::query("UPDATE audit_log SET actor_user_id = NULL, metadata_json = '{}' WHERE installation_id = $1 AND workspace_id = $2").bind(installation).bind(workspace.id.to_string()).execute(&mut *connection).await?;
    sqlx::query(
        "DELETE FROM workspaces WHERE installation_id = $1 AND id = $2 AND state = 'deleted'",
    )
    .bind(installation)
    .bind(workspace.id.to_string())
    .execute(connection)
    .await?;
    Ok(())
}

async fn job_and_audit_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace: &Workspace,
    transition: StoredTransition,
    actor: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    if transition.enqueues_reconcile() {
        sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES (?1, ?2, 'reconcile_workspace', ?3, ?4, 'pending', ?5, NULL, NULL, 0, ?5, ?5)")
            .bind(Uuid::now_v7().to_string()).bind(installation_id).bind(workspace.id.to_string())
            .bind(serde_json::json!({"generation": workspace.generation, "action": transition.as_str()}).to_string())
            .bind(now).execute(&mut *connection).await?;
    }
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, '{}', ?7)")
        .bind(Uuid::now_v7().to_string()).bind(installation_id).bind(actor.to_string())
        .bind(workspace.organization_id.to_string()).bind(workspace.id.to_string())
        .bind(format!("workspace.{}", transition.as_str())).bind(now).execute(&mut *connection).await?;
    super::workspace_events::insert_sqlite(
        connection,
        installation_id,
        workspace,
        Some(transition.as_str()),
        now,
    )
    .await?;
    Ok(())
}

async fn job_and_audit_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &Workspace,
    transition: StoredTransition,
    actor: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    if transition.enqueues_reconcile() {
        sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES ($1, $2, 'reconcile_workspace', $3, $4, 'pending', $5, NULL, NULL, 0, $5, $5)")
            .bind(Uuid::now_v7().to_string()).bind(installation_id).bind(workspace.id.to_string())
            .bind(serde_json::json!({"generation": workspace.generation, "action": transition.as_str()}).to_string())
            .bind(now).execute(&mut *connection).await?;
    }
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES ($1, $2, $3, $4, $5, $6, '{}', $7)")
        .bind(Uuid::now_v7().to_string()).bind(installation_id).bind(actor.to_string())
        .bind(workspace.organization_id.to_string()).bind(workspace.id.to_string())
        .bind(format!("workspace.{}", transition.as_str())).bind(now).execute(&mut *connection).await?;
    super::workspace_events::insert_postgres(
        connection,
        installation_id,
        workspace,
        Some(transition.as_str()),
        now,
    )
    .await?;
    Ok(())
}

fn as_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::InvalidWorkspace)
}
