use sqlx::{PgConnection, SqliteConnection};
use uuid::Uuid;

use crate::storage::StorageError;

pub(super) async fn audit_api_key_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    actor: Uuid,
    key_id: Uuid,
    action: &str,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES (?1, ?2, ?3, NULL, NULL, ?4, ?5, ?6)")
        .bind(Uuid::now_v7().to_string()).bind(installation).bind(actor.to_string()).bind(action)
        .bind(serde_json::json!({"api_key_id": key_id}).to_string()).bind(now)
        .execute(connection).await?;
    Ok(())
}

pub(super) async fn audit_api_key_postgres(
    connection: &mut PgConnection,
    installation: &str,
    actor: Uuid,
    key_id: Uuid,
    action: &str,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES ($1, $2, $3, NULL, NULL, $4, $5, $6)")
        .bind(Uuid::now_v7().to_string()).bind(installation).bind(actor.to_string()).bind(action)
        .bind(serde_json::json!({"api_key_id": key_id}).to_string()).bind(now)
        .execute(connection).await?;
    Ok(())
}

pub(super) async fn audit_admin_api_key_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    actor: Uuid,
    target_user_id: Uuid,
    key_id: Uuid,
    reason: &str,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES (?1, ?2, ?3, NULL, NULL, ?4, ?5, ?6)")
        .bind(Uuid::now_v7().to_string()).bind(installation).bind(actor.to_string())
        .bind("user.api_key.admin_revoke")
        .bind(serde_json::json!({"target_user_id": target_user_id, "api_key_id": key_id, "reason": reason}).to_string()).bind(now)
        .execute(connection).await?;
    Ok(())
}

pub(super) async fn audit_admin_api_key_postgres(
    connection: &mut PgConnection,
    installation: &str,
    actor: Uuid,
    target_user_id: Uuid,
    key_id: Uuid,
    reason: &str,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES ($1, $2, $3, NULL, NULL, $4, $5, $6)")
        .bind(Uuid::now_v7().to_string()).bind(installation).bind(actor.to_string())
        .bind("user.api_key.admin_revoke")
        .bind(serde_json::json!({"target_user_id": target_user_id, "api_key_id": key_id, "reason": reason}).to_string()).bind(now)
        .execute(connection).await?;
    Ok(())
}
