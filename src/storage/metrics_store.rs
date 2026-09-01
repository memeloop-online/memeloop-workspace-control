use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sqlx::{Row, postgres::PgRow, sqlite::SqliteRow};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::quota::Resources;

use super::{Database, StorageError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct JobCounts {
    pub pending: i64,
    pub running: i64,
    pub completed: i64,
    pub failed: i64,
    /// Unix timestamp of the oldest pending durable job, if any.
    pub oldest_pending_created_at: Option<i64>,
    /// Highest attempt count among work that can still execute.
    pub max_active_attempts: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceMetrics {
    pub states: BTreeMap<String, i64>,
    pub users: Vec<UserWorkspaceMetrics>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserWorkspaceMetrics {
    pub user_id: Uuid,
    pub states: BTreeMap<String, i64>,
    pub resources: Resources,
}

impl Database {
    pub async fn job_counts(&self) -> Result<JobCounts, StorageError> {
        let row = match self {
            Self::Sqlite { pool, installation_id } => sqlx::query("SELECT SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END) pending, SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END) running, SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) completed, SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END) failed, MIN(CASE WHEN status = 'pending' THEN created_at END) oldest_pending_created_at, MAX(CASE WHEN status IN ('pending', 'running') THEN attempts ELSE 0 END) max_active_attempts FROM jobs WHERE installation_id = ?1")
                .bind(installation_id.as_str()).fetch_one(pool).await?,
            Self::Postgres { pool, installation_id } => return decode_job_counts(sqlx::query("SELECT CAST(SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END) AS BIGINT) pending, CAST(SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END) AS BIGINT) running, CAST(SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) AS BIGINT) completed, CAST(SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END) AS BIGINT) failed, MIN(CASE WHEN status = 'pending' THEN created_at END) oldest_pending_created_at, CAST(MAX(CASE WHEN status IN ('pending', 'running') THEN attempts ELSE 0 END) AS BIGINT) max_active_attempts FROM jobs WHERE installation_id = $1")
                .bind(installation_id.as_str()).fetch_one(pool).await?),
        };
        decode_job_counts(row)
    }

    pub async fn workspace_metrics(&self) -> Result<WorkspaceMetrics, StorageError> {
        let sql = "SELECT w.owner_id, w.state, COUNT(*) AS workspace_count, \
            CAST(SUM(w.cpu_millis) AS BIGINT) AS cpu_millis, \
            CAST(SUM(w.memory_mib) AS BIGINT) AS memory_mib, \
            CAST(SUM(w.gpu_count) AS BIGINT) AS gpu_count, \
            CAST(SUM(w.disk_gib) AS BIGINT) AS disk_gib \
            FROM workspaces w \
            WHERE w.installation_id = {install} AND w.state <> 'deleted' \
            GROUP BY w.owner_id, w.state ORDER BY w.owner_id, w.state";
        let rows = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(&sql.replace("{install}", "?1"))
                .bind(installation_id.as_str())
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(decode_sqlite_workspace_metric)
                .collect::<Result<Vec<_>, _>>()?,
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(&sql.replace("{install}", "$1"))
                .bind(installation_id.as_str())
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(decode_postgres_workspace_metric)
                .collect::<Result<Vec<_>, _>>()?,
        };
        Ok(aggregate_workspace_metrics(rows))
    }
}

#[derive(Debug)]
struct WorkspaceMetricRow {
    user_id: Uuid,
    state: String,
    workspace_count: i64,
    resources: Resources,
}

fn decode_sqlite_workspace_metric(row: SqliteRow) -> Result<WorkspaceMetricRow, StorageError> {
    decode_workspace_metric(&row)
}

fn decode_postgres_workspace_metric(row: PgRow) -> Result<WorkspaceMetricRow, StorageError> {
    decode_workspace_metric(&row)
}

fn decode_workspace_metric<R: Row>(row: &R) -> Result<WorkspaceMetricRow, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    Ok(WorkspaceMetricRow {
        user_id: Uuid::parse_str(&row.try_get::<String, _>("owner_id")?)?,
        state: row.try_get("state")?,
        workspace_count: row.try_get("workspace_count")?,
        resources: Resources {
            cpu_millis: u64::try_from(row.try_get::<i64, _>("cpu_millis")?)
                .map_err(|_| StorageError::InvalidWorkspace)?,
            memory_mib: u64::try_from(row.try_get::<i64, _>("memory_mib")?)
                .map_err(|_| StorageError::InvalidWorkspace)?,
            gpu_count: u32::try_from(row.try_get::<i64, _>("gpu_count")?)
                .map_err(|_| StorageError::InvalidWorkspace)?,
            disk_gib: u64::try_from(row.try_get::<i64, _>("disk_gib")?)
                .map_err(|_| StorageError::InvalidWorkspace)?,
        },
    })
}

fn aggregate_workspace_metrics(rows: Vec<WorkspaceMetricRow>) -> WorkspaceMetrics {
    let mut states = BTreeMap::new();
    let mut users = BTreeMap::<Uuid, UserWorkspaceMetrics>::new();
    for row in rows {
        *states.entry(row.state.clone()).or_default() += row.workspace_count;
        let user = users
            .entry(row.user_id)
            .or_insert_with(|| UserWorkspaceMetrics {
                user_id: row.user_id,
                states: BTreeMap::new(),
                resources: Resources::default(),
            });
        *user.states.entry(row.state).or_default() += row.workspace_count;
        user.resources.cpu_millis = user
            .resources
            .cpu_millis
            .saturating_add(row.resources.cpu_millis);
        user.resources.memory_mib = user
            .resources
            .memory_mib
            .saturating_add(row.resources.memory_mib);
        user.resources.gpu_count = user
            .resources
            .gpu_count
            .saturating_add(row.resources.gpu_count);
        user.resources.disk_gib = user
            .resources
            .disk_gib
            .saturating_add(row.resources.disk_gib);
    }
    WorkspaceMetrics {
        states,
        users: users.into_values().collect(),
    }
}

fn decode_job_counts<R: Row>(row: R) -> Result<JobCounts, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    Ok(JobCounts {
        pending: row
            .try_get::<Option<i64>, _>("pending")?
            .unwrap_or_default(),
        running: row
            .try_get::<Option<i64>, _>("running")?
            .unwrap_or_default(),
        completed: row
            .try_get::<Option<i64>, _>("completed")?
            .unwrap_or_default(),
        failed: row.try_get::<Option<i64>, _>("failed")?.unwrap_or_default(),
        oldest_pending_created_at: row.try_get("oldest_pending_created_at")?,
        max_active_attempts: row
            .try_get::<Option<i64>, _>("max_active_attempts")?
            .unwrap_or_default(),
    })
}
