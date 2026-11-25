use std::collections::BTreeMap;

use sqlx::Row;
use uuid::Uuid;

use crate::quota::QuotaResources;

use super::{Database, StorageError, template_filter_sql};

/// Organization-wide requested capacity and lifecycle counts.
///
/// This is deliberately a small database aggregate rather than a collection of
/// [`Workspace`](crate::workspaces::Workspace)s, as organizations can contain
/// many thousands of workspaces.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceUsageSummary {
    pub total_count: u64,
    pub requested: QuotaResources,
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
    let sqlite_sql = usage_states_sql(&sqlite_filter, false);
    let postgres_sql = usage_states_sql(&postgres_filter, true);

    match database {
        Database::Sqlite {
            pool,
            installation_id,
        } => {
            let mut query = sqlx::query(&sqlite_sql)
                .bind(installation_id.as_str())
                .bind(organization_id.to_string());
            for template_id in allowed_template_ids.unwrap_or_default() {
                query = query.bind(template_id.to_string());
            }
            decode_usage_summary(query.fetch_all(pool).await?)
        }
        Database::Postgres {
            pool,
            installation_id,
        } => {
            let mut query = sqlx::query(&postgres_sql)
                .bind(installation_id.as_str())
                .bind(organization_id.to_string());
            for template_id in allowed_template_ids.unwrap_or_default() {
                query = query.bind(template_id.to_string());
            }
            decode_usage_summary(query.fetch_all(pool).await?)
        }
    }
}

fn usage_states_sql(filter: &str, postgres: bool) -> String {
    let cast = |value: &str| {
        if postgres {
            format!("CAST({value} AS BIGINT)")
        } else {
            value.to_owned()
        }
    };
    format!(
        "SELECT state, {total_count} AS workspace_count, {cpu_millis} AS cpu_millis, \
         {memory_mib} AS memory_mib, {gpu_count} AS gpu_count, {disk_gib} AS disk_gib, \
         {temporary_storage_gib} AS temporary_storage_gib \
         FROM workspaces WHERE {filter} GROUP BY state",
        total_count = cast("COUNT(*)"),
        cpu_millis = cast("COALESCE(SUM(cpu_millis), 0)"),
        memory_mib = cast("COALESCE(SUM(memory_mib), 0)"),
        gpu_count = cast("COALESCE(SUM(gpu_count), 0)"),
        disk_gib = cast("COALESCE(SUM(disk_gib), 0)"),
        temporary_storage_gib = cast("COALESCE(SUM(temporary_storage_gib), 0)"),
    )
}

fn usage_filter_sql(
    installation: &str,
    organization: &str,
    allowed_template_ids: Option<&[Uuid]>,
    placeholder_style: char,
) -> String {
    let templates = template_filter_sql(allowed_template_ids, placeholder_style, 3);
    format!(
        "installation_id = {installation} AND organization_id = {organization} AND state <> 'deleted'{templates}"
    )
}

fn decode_usage_summary<R: Row>(rows: Vec<R>) -> Result<WorkspaceUsageSummary, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
    i64: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
{
    let mut summary = WorkspaceUsageSummary::default();
    for row in rows {
        let read_u64 = |column| {
            u64::try_from(row.try_get::<i64, _>(column)?)
                .map_err(|_| StorageError::InvalidWorkspace)
        };
        let state: String = row.try_get("state")?;
        let count = read_u64("workspace_count")?;
        let active_runtime = usage_state_has_live_runtime(&state);
        let requested = QuotaResources {
            cpu_millis: read_u64("cpu_millis")?,
            memory_mib: read_u64("memory_mib")?,
            gpu_count: u32::try_from(row.try_get::<i64, _>("gpu_count")?)
                .map_err(|_| StorageError::InvalidWorkspace)?,
            disk_gib: read_u64("disk_gib")?,
            temporary_storage_gib: if active_runtime {
                read_u64("temporary_storage_gib")?
            } else {
                0
            },
        };
        summary.total_count = summary
            .total_count
            .checked_add(count)
            .ok_or(StorageError::InvalidWorkspace)?;
        summary.requested = summary
            .requested
            .checked_add(requested)
            .map_err(|_| StorageError::InvalidWorkspace)?;
        if active_runtime {
            summary.active_count = summary
                .active_count
                .checked_add(count)
                .ok_or(StorageError::InvalidWorkspace)?;
        }
        summary.state_counts.insert(state, count);
    }
    Ok(summary)
}

fn usage_state_has_live_runtime(state: &str) -> bool {
    matches!(
        state,
        "provisioning" | "ready" | "starting" | "restarting" | "stopping" | "deleting"
    )
}

pub(super) async fn workspace_page_summary(
    database: &Database,
    organization_id: Uuid,
    search: &str,
    pattern: &str,
    allowed_template_ids: Option<&[Uuid]>,
) -> Result<WorkspaceUsageSummary, StorageError> {
    let sqlite_filter = workspace_filter_sql("?1", "?2", "?3", "?4")
        + &template_filter_sql(allowed_template_ids, '?', 5);
    let postgres_filter = workspace_filter_sql("$1", "$2", "$3", "$4")
        + &template_filter_sql(allowed_template_ids, '$', 5);
    match database {
        Database::Sqlite {
            pool,
            installation_id,
        } => {
            let sql = usage_states_sql(&sqlite_filter, false);
            let mut query = sqlx::query(&sql)
                .bind(installation_id.as_str())
                .bind(organization_id.to_string())
                .bind(search)
                .bind(pattern);
            for id in allowed_template_ids.unwrap_or_default() {
                query = query.bind(id.to_string());
            }
            decode_usage_summary(query.fetch_all(pool).await?)
        }
        Database::Postgres {
            pool,
            installation_id,
        } => {
            let sql = usage_states_sql(&postgres_filter, true);
            let mut query = sqlx::query(&sql)
                .bind(installation_id.as_str())
                .bind(organization_id.to_string())
                .bind(search)
                .bind(pattern);
            for id in allowed_template_ids.unwrap_or_default() {
                query = query.bind(id.to_string());
            }
            decode_usage_summary(query.fetch_all(pool).await?)
        }
    }
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
