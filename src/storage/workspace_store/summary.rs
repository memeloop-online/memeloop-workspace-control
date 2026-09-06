use std::collections::BTreeMap;

use sqlx::Row;
use uuid::Uuid;

use crate::quota::Resources;

use super::{Database, StorageError};

#[derive(Debug, Clone)]
pub(super) struct WorkspacePageSummary {
    pub(super) total_count: u64,
    pub(super) requested: Resources,
    pub(super) state_counts: BTreeMap<String, u64>,
}

pub(super) async fn workspace_page_summary(
    database: &Database,
    organization_id: Uuid,
    search: &str,
    pattern: &str,
) -> Result<WorkspacePageSummary, StorageError> {
    let sqlite_filter = workspace_filter_sql("?1", "?2", "?3", "?4");
    let sqlite_sql = format!(
        "SELECT COUNT(*) AS total_count, \
        COALESCE(SUM(cpu_millis), 0) AS cpu_millis, \
        COALESCE(SUM(memory_mib), 0) AS memory_mib, \
        COALESCE(SUM(gpu_count), 0) AS gpu_count, \
        COALESCE(SUM(disk_gib), 0) AS disk_gib \
        FROM workspaces WHERE {sqlite_filter}"
    );
    let postgres_filter = workspace_filter_sql("$1", "$2", "$3", "$4");
    let postgres_sql = format!(
        "SELECT COUNT(*) AS total_count, \
        CAST(COALESCE(SUM(cpu_millis), 0) AS BIGINT) AS cpu_millis, \
        CAST(COALESCE(SUM(memory_mib), 0) AS BIGINT) AS memory_mib, \
        CAST(COALESCE(SUM(gpu_count), 0) AS BIGINT) AS gpu_count, \
        CAST(COALESCE(SUM(disk_gib), 0) AS BIGINT) AS disk_gib \
        FROM workspaces WHERE {postgres_filter}"
    );
    let aggregate = match database {
        Database::Sqlite {
            pool,
            installation_id,
        } => decode_summary(
            &sqlx::query(&sqlite_sql)
                .bind(installation_id.as_str())
                .bind(organization_id.to_string())
                .bind(search)
                .bind(pattern)
                .fetch_one(pool)
                .await?,
        ),
        Database::Postgres {
            pool,
            installation_id,
        } => decode_summary(
            &sqlx::query(&postgres_sql)
                .bind(installation_id.as_str())
                .bind(organization_id.to_string())
                .bind(search)
                .bind(pattern)
                .fetch_one(pool)
                .await?,
        ),
    }?;
    let state_counts =
        workspace_page_state_counts(database, organization_id, search, pattern).await?;
    Ok(WorkspacePageSummary {
        total_count: aggregate.total_count,
        requested: aggregate.requested,
        state_counts,
    })
}

async fn workspace_page_state_counts(
    database: &Database,
    organization_id: Uuid,
    search: &str,
    pattern: &str,
) -> Result<BTreeMap<String, u64>, StorageError> {
    let sqlite_sql = format!(
        "SELECT state, COUNT(*) AS workspace_count FROM workspaces WHERE {} GROUP BY state",
        workspace_filter_sql("?1", "?2", "?3", "?4")
    );
    let postgres_sql = format!(
        "SELECT state, COUNT(*) AS workspace_count FROM workspaces WHERE {} GROUP BY state",
        workspace_filter_sql("$1", "$2", "$3", "$4")
    );
    match database {
        Database::Sqlite {
            pool,
            installation_id,
        } => decode_state_counts(
            sqlx::query(&sqlite_sql)
                .bind(installation_id.as_str())
                .bind(organization_id.to_string())
                .bind(search)
                .bind(pattern)
                .fetch_all(pool)
                .await?,
        ),
        Database::Postgres {
            pool,
            installation_id,
        } => decode_state_counts(
            sqlx::query(&postgres_sql)
                .bind(installation_id.as_str())
                .bind(organization_id.to_string())
                .bind(search)
                .bind(pattern)
                .fetch_all(pool)
                .await?,
        ),
    }
}

fn decode_summary<R: Row>(row: &R) -> Result<WorkspacePageSummary, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    i64: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
{
    let read_u64 = |column| {
        u64::try_from(row.try_get::<i64, _>(column)?).map_err(|_| StorageError::InvalidWorkspace)
    };
    Ok(WorkspacePageSummary {
        total_count: read_u64("total_count")?,
        requested: Resources {
            cpu_millis: read_u64("cpu_millis")?,
            memory_mib: read_u64("memory_mib")?,
            gpu_count: u32::try_from(row.try_get::<i64, _>("gpu_count")?)
                .map_err(|_| StorageError::InvalidWorkspace)?,
            disk_gib: read_u64("disk_gib")?,
        },
        state_counts: BTreeMap::new(),
    })
}

fn decode_state_counts<R: Row>(rows: Vec<R>) -> Result<BTreeMap<String, u64>, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
    i64: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
{
    rows.into_iter()
        .map(|row| {
            let count = u64::try_from(row.try_get::<i64, _>("workspace_count")?)
                .map_err(|_| StorageError::InvalidWorkspace)?;
            Ok((row.try_get("state")?, count))
        })
        .collect()
}

pub(super) fn workspace_filter_sql(
    installation: &str,
    organization: &str,
    search: &str,
    pattern: &str,
) -> String {
    format!(
        "installation_id = {installation} AND organization_id = {organization} AND state <> 'deleted' \
         AND ({search} = '' OR LOWER(name) LIKE {pattern} ESCAPE '\\' \
            OR LOWER(short_id) LIKE {pattern} ESCAPE '\\' \
            OR LOWER(state) LIKE {pattern} ESCAPE '\\' \
            OR LOWER(image) LIKE {pattern} ESCAPE '\\' \
            OR LOWER(template_snapshot_yaml) LIKE {pattern} ESCAPE '\\')"
    )
}
