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

/// Organization-wide requested capacity and lifecycle counts.
///
/// This is deliberately a small database aggregate rather than a collection of
/// [`Workspace`](crate::workspaces::Workspace)s, as organizations can contain
/// many thousands of workspaces.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceUsageSummary {
    pub total_count: u64,
    pub requested: Resources,
    pub state_counts: BTreeMap<String, u64>,
    /// Workspaces which can still have a live runtime. Stopped and failed
    /// workspaces are excluded; deleting and stopping workspaces remain active
    /// until reconciliation has removed their runtime.
    pub active_count: u64,
}

pub(super) async fn workspace_usage_summary(
    database: &Database,
    organization_id: Uuid,
    allowed_template_ids: Option<&[Uuid]>,
) -> Result<WorkspaceUsageSummary, StorageError> {
    let Some(allowed_template_ids) = allowed_template_ids else {
        return workspace_usage_summary_for_templates(database, organization_id, None).await;
    };
    if allowed_template_ids.is_empty() {
        return Ok(WorkspaceUsageSummary::default());
    }
    workspace_usage_summary_for_templates(database, organization_id, Some(allowed_template_ids))
        .await
}

async fn workspace_usage_summary_for_templates(
    database: &Database,
    organization_id: Uuid,
    allowed_template_ids: Option<&[Uuid]>,
) -> Result<WorkspaceUsageSummary, StorageError> {
    let sqlite_filter = usage_filter_sql("?1", "?2", allowed_template_ids, '?');
    let postgres_filter = usage_filter_sql("$1", "$2", allowed_template_ids, '$');
    let sqlite_aggregate_sql = usage_aggregate_sql(&sqlite_filter, false);
    let postgres_aggregate_sql = usage_aggregate_sql(&postgres_filter, true);
    let sqlite_states_sql = usage_states_sql(&sqlite_filter);
    let postgres_states_sql = usage_states_sql(&postgres_filter);

    match database {
        Database::Sqlite {
            pool,
            installation_id,
        } => {
            let mut aggregate = sqlx::query(&sqlite_aggregate_sql)
                .bind(installation_id.as_str())
                .bind(organization_id.to_string());
            let mut states = sqlx::query(&sqlite_states_sql)
                .bind(installation_id.as_str())
                .bind(organization_id.to_string());
            for template_id in allowed_template_ids.unwrap_or_default() {
                aggregate = aggregate.bind(template_id.to_string());
                states = states.bind(template_id.to_string());
            }
            let aggregate = decode_usage_summary(&aggregate.fetch_one(pool).await?)?;
            let state_counts = decode_state_counts(states.fetch_all(pool).await?)?;
            Ok(WorkspaceUsageSummary {
                state_counts,
                ..aggregate
            })
        }
        Database::Postgres {
            pool,
            installation_id,
        } => {
            let mut aggregate = sqlx::query(&postgres_aggregate_sql)
                .bind(installation_id.as_str())
                .bind(organization_id.to_string());
            let mut states = sqlx::query(&postgres_states_sql)
                .bind(installation_id.as_str())
                .bind(organization_id.to_string());
            for template_id in allowed_template_ids.unwrap_or_default() {
                aggregate = aggregate.bind(template_id.to_string());
                states = states.bind(template_id.to_string());
            }
            let aggregate = decode_usage_summary(&aggregate.fetch_one(pool).await?)?;
            let state_counts = decode_state_counts(states.fetch_all(pool).await?)?;
            Ok(WorkspaceUsageSummary {
                state_counts,
                ..aggregate
            })
        }
    }
}

fn usage_aggregate_sql(filter: &str, postgres: bool) -> String {
    let cast = |value: &str| {
        if postgres {
            format!("CAST({value} AS BIGINT)")
        } else {
            value.to_owned()
        }
    };
    format!(
        "SELECT {total_count} AS total_count, {cpu_millis} AS cpu_millis, \
         {memory_mib} AS memory_mib, {gpu_count} AS gpu_count, {disk_gib} AS disk_gib, \
         {active_count} AS active_count FROM workspaces WHERE {filter}",
        total_count = cast("COUNT(*)"),
        cpu_millis = cast("COALESCE(SUM(cpu_millis), 0)"),
        memory_mib = cast("COALESCE(SUM(memory_mib), 0)"),
        gpu_count = cast("COALESCE(SUM(gpu_count), 0)"),
        disk_gib = cast("COALESCE(SUM(disk_gib), 0)"),
        active_count = cast(
            "COALESCE(SUM(CASE WHEN state NOT IN ('stopped', 'failed') THEN 1 ELSE 0 END), 0)"
        ),
    )
}

fn usage_states_sql(filter: &str) -> String {
    format!(
        "SELECT state, COUNT(*) AS workspace_count FROM workspaces WHERE {filter} GROUP BY state"
    )
}

fn usage_filter_sql(
    installation: &str,
    organization: &str,
    allowed_template_ids: Option<&[Uuid]>,
    placeholder_style: char,
) -> String {
    let mut filter = format!(
        "installation_id = {installation} AND organization_id = {organization} AND state <> 'deleted'"
    );
    if let Some(template_ids) = allowed_template_ids {
        let placeholders = template_ids
            .iter()
            .enumerate()
            .map(|(index, _)| match placeholder_style {
                '?' => format!("?{}", index + 3),
                '$' => format!("${}", index + 3),
                _ => unreachable!("only SQLite and PostgreSQL placeholder styles are supported"),
            })
            .collect::<Vec<_>>()
            .join(",");
        filter.push_str(&format!(" AND template_id IN ({placeholders})"));
    }
    filter
}

fn decode_usage_summary<R: Row>(row: &R) -> Result<WorkspaceUsageSummary, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    i64: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
{
    let read_u64 = |column| {
        u64::try_from(row.try_get::<i64, _>(column)?).map_err(|_| StorageError::InvalidWorkspace)
    };
    Ok(WorkspaceUsageSummary {
        total_count: read_u64("total_count")?,
        requested: Resources {
            cpu_millis: read_u64("cpu_millis")?,
            memory_mib: read_u64("memory_mib")?,
            gpu_count: u32::try_from(row.try_get::<i64, _>("gpu_count")?)
                .map_err(|_| StorageError::InvalidWorkspace)?,
            disk_gib: read_u64("disk_gib")?,
        },
        state_counts: BTreeMap::new(),
        active_count: read_u64("active_count")?,
    })
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
