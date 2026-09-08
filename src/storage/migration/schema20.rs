//! One-release schema 20 bridge.
//!
//! Schema 20 persisted Kubernetes placement per workspace. Schema 21 makes the
//! product Namespace an invariant, so the placement columns are validated and
//! removed atomically. This module is intentionally isolated for deletion after
//! the rollout has crossed schema 20 everywhere.

use sqlx::{PgConnection, Row, SqliteConnection};
use uuid::Uuid;

use crate::{
    config::InstallationId, storage::StorageError, workspace_runtime::workspace_short_id_for,
};

pub(super) async fn upgrade_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &InstallationId,
) -> Result<(), StorageError> {
    let rows = sqlx::query(
        "SELECT id, installation_id, short_id, runtime_namespace_scope, runtime_namespace \
         FROM workspaces",
    )
    .fetch_all(&mut *connection)
    .await?;
    validate_rows(&rows, installation_id)?;
    sqlx::query("ALTER TABLE workspaces DROP COLUMN runtime_namespace_scope")
        .execute(&mut *connection)
        .await?;
    sqlx::query("ALTER TABLE workspaces DROP COLUMN runtime_namespace")
        .execute(&mut *connection)
        .await?;
    Ok(())
}

pub(super) async fn upgrade_postgres(
    connection: &mut PgConnection,
    installation_id: &InstallationId,
) -> Result<(), StorageError> {
    let rows = sqlx::query(
        "SELECT id, installation_id, short_id, runtime_namespace_scope, runtime_namespace \
         FROM workspaces",
    )
    .fetch_all(&mut *connection)
    .await?;
    validate_rows(&rows, installation_id)?;
    sqlx::query(
        "ALTER TABLE workspaces DROP COLUMN runtime_namespace_scope, \
         DROP COLUMN runtime_namespace",
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}

fn validate_rows<R>(rows: &[R], installation_id: &InstallationId) -> Result<(), StorageError>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
{
    for row in rows {
        let row_installation: String = row.try_get("installation_id")?;
        let id = Uuid::parse_str(&row.try_get::<String, _>("id")?)?;
        let short_id: String = row.try_get("short_id")?;
        let scope: String = row.try_get("runtime_namespace_scope")?;
        let namespace: String = row.try_get("runtime_namespace")?;
        if row_installation != installation_id.as_str()
            || short_id != workspace_short_id_for(id)
            || !matches!(scope.as_str(), "dedicated" | "shared")
            || !valid_dns_label(&namespace)
        {
            return Err(StorageError::InvalidWorkspace);
        }
    }
    Ok(())
}

fn valid_dns_label(value: &str) -> bool {
    let edge = |character: char| character.is_ascii_lowercase() || character.is_ascii_digit();
    !value.is_empty()
        && value.len() <= 63
        && value
            .chars()
            .all(|character| edge(character) || character == '-')
        && value.chars().next().is_some_and(edge)
        && value.chars().last().is_some_and(edge)
}
