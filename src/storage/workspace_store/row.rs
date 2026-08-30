use sqlx::{Row, postgres::PgRow, sqlite::SqliteRow};
use uuid::Uuid;

use crate::{
    storage::StorageError,
    templates::WorkspaceTemplateDocument,
    workspaces::{Workspace, WorkspaceState},
};

pub(super) const WORKSPACE_COLUMNS: &str = "id, short_id, organization_id, owner_id, name, \
    template_id, template_snapshot_yaml, state, generation, created_at, updated_at";

pub(crate) fn select_workspace_sql(installation: &str, id: &str) -> String {
    format!(
        "SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE installation_id = {installation} AND id = {id}"
    )
}

pub(crate) fn select_workspace_by_short_id_sql(installation: &str, short_id: &str) -> String {
    format!(
        "SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE installation_id = {installation} AND short_id = {short_id}"
    )
}

pub(crate) fn decode_sqlite(row: SqliteRow) -> Result<Workspace, StorageError> {
    decode_workspace(&row)
}

pub(crate) fn decode_postgres(row: PgRow) -> Result<Workspace, StorageError> {
    decode_workspace(&row)
}

fn decode_workspace<R: Row>(row: &R) -> Result<Workspace, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
    i64: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
{
    let state: String = row.try_get("state")?;
    let template_id: Option<String> = row.try_get("template_id")?;
    let yaml: String = row.try_get("template_snapshot_yaml")?;
    let document =
        WorkspaceTemplateDocument::parse(&yaml).map_err(|_| StorageError::InvalidWorkspace)?;
    Ok(Workspace {
        id: Uuid::parse_str(&row.try_get::<String, _>("id")?)?,
        short_id: row.try_get("short_id")?,
        organization_id: Uuid::parse_str(&row.try_get::<String, _>("organization_id")?)?,
        owner_id: Uuid::parse_str(&row.try_get::<String, _>("owner_id")?)?,
        name: row.try_get("name")?,
        template_id: template_id.map(|id| Uuid::parse_str(&id)).transpose()?,
        template: document.spec,
        state: WorkspaceState::from_database(&state)
            .ok_or(StorageError::UnknownWorkspaceState(state))?,
        generation: u64::try_from(row.try_get::<i64, _>("generation")?)
            .map_err(|_| StorageError::InvalidWorkspace)?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}
