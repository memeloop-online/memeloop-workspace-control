use sqlx::{Row, postgres::PgRow, sqlite::SqliteRow};
use uuid::Uuid;

use super::{WorkspaceTemplate, parse_document};
use crate::storage::StorageError;

pub(super) enum TemplateDatabaseRow {
    Sqlite(SqliteRow),
    Postgres(PgRow),
}

pub(super) fn decode_optional_template(
    row: Option<TemplateDatabaseRow>,
) -> Result<WorkspaceTemplate, StorageError> {
    match row {
        Some(TemplateDatabaseRow::Sqlite(row)) => decode_template(row),
        Some(TemplateDatabaseRow::Postgres(row)) => decode_template(row),
        None => Err(StorageError::TemplateNotFound),
    }
}

pub(super) fn decode_template<R: Row>(row: R) -> Result<WorkspaceTemplate, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    let organization: Option<String> = row.try_get("organization_id")?;
    let yaml: String = row.try_get("template_yaml")?;
    let document = parse_document(&yaml)?;
    Ok(WorkspaceTemplate {
        id: Uuid::parse_str(&row.try_get::<String, _>("id")?)?,
        organization_id: organization.map(|id| Uuid::parse_str(&id)).transpose()?,
        name: document.metadata.name,
        template: document.spec,
        yaml,
        enabled: row.try_get::<i64, _>("enabled")? != 0,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}
