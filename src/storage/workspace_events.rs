use sqlx::{PgConnection, SqliteConnection};
use uuid::Uuid;

use crate::workspaces::Workspace;

use super::StorageError;

pub(super) async fn insert_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace: &Workspace,
    action: Option<&str>,
    now: i64,
) -> Result<(), StorageError> {
    let event_id = Uuid::now_v7();
    sqlx::query("INSERT INTO events (id, installation_id, organization_id, workspace_id, kind, payload_json, created_at) VALUES (?1, ?2, ?3, ?4, 'workspace.state_changed', ?5, ?6)")
        .bind(event_id.to_string())
        .bind(installation_id)
        .bind(workspace.organization_id.to_string())
        .bind(workspace.id.to_string())
        .bind(payload(workspace, action).to_string())
        .bind(now)
        .execute(&mut *connection)
        .await?;
    super::webhook_store::enqueue_sqlite(
        connection,
        installation_id,
        workspace.organization_id,
        workspace.id,
        event_id,
        "workspace.state_changed",
        now,
    )
    .await?;
    Ok(())
}

pub(super) async fn insert_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &Workspace,
    action: Option<&str>,
    now: i64,
) -> Result<(), StorageError> {
    let event_id = Uuid::now_v7();
    sqlx::query("INSERT INTO events (id, installation_id, organization_id, workspace_id, kind, payload_json, created_at) VALUES ($1, $2, $3, $4, 'workspace.state_changed', $5, $6)")
        .bind(event_id.to_string())
        .bind(installation_id)
        .bind(workspace.organization_id.to_string())
        .bind(workspace.id.to_string())
        .bind(payload(workspace, action).to_string())
        .bind(now)
        .execute(&mut *connection)
        .await?;
    super::webhook_store::enqueue_postgres(
        connection,
        installation_id,
        workspace.organization_id,
        workspace.id,
        event_id,
        "workspace.state_changed",
        now,
    )
    .await?;
    sqlx::query("SELECT pg_notify('mwc_events', $1)")
        .bind(installation_id)
        .execute(&mut *connection)
        .await?;
    Ok(())
}

pub(super) async fn insert_image_updated_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace: &Workspace,
    now: i64,
) -> Result<(), StorageError> {
    let event_id = Uuid::now_v7();
    sqlx::query("INSERT INTO events (id, installation_id, organization_id, workspace_id, kind, payload_json, created_at) VALUES (?1, ?2, ?3, ?4, 'workspace.image_updated', ?5, ?6)")
        .bind(event_id.to_string())
        .bind(installation_id)
        .bind(workspace.organization_id.to_string())
        .bind(workspace.id.to_string())
        .bind(image_updated_payload(workspace).to_string())
        .bind(now)
        .execute(&mut *connection)
        .await?;
    super::webhook_store::enqueue_sqlite(
        connection,
        installation_id,
        workspace.organization_id,
        workspace.id,
        event_id,
        "workspace.image_updated",
        now,
    )
    .await
}

pub(super) async fn insert_image_updated_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &Workspace,
    now: i64,
) -> Result<(), StorageError> {
    let event_id = Uuid::now_v7();
    sqlx::query("INSERT INTO events (id, installation_id, organization_id, workspace_id, kind, payload_json, created_at) VALUES ($1, $2, $3, $4, 'workspace.image_updated', $5, $6)")
        .bind(event_id.to_string())
        .bind(installation_id)
        .bind(workspace.organization_id.to_string())
        .bind(workspace.id.to_string())
        .bind(image_updated_payload(workspace).to_string())
        .bind(now)
        .execute(&mut *connection)
        .await?;
    super::webhook_store::enqueue_postgres(
        connection,
        installation_id,
        workspace.organization_id,
        workspace.id,
        event_id,
        "workspace.image_updated",
        now,
    )
    .await?;
    sqlx::query("SELECT pg_notify('mwc_events', $1)")
        .bind(installation_id)
        .execute(&mut *connection)
        .await?;
    Ok(())
}

fn payload(workspace: &Workspace, action: Option<&str>) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "state": workspace.state.as_str(),
        "generation": workspace.generation,
    });
    if let Some(action) = action {
        payload["action"] = action.into();
    }
    payload
}

fn image_updated_payload(workspace: &Workspace) -> serde_json::Value {
    serde_json::json!({
        "image": workspace.template.image,
        "generation": workspace.generation,
    })
}
