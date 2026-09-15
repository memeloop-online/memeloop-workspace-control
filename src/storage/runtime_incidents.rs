use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sqlx::{Postgres, QueryBuilder, Row, Sqlite, postgres::PgRow, sqlite::SqliteRow};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{Database, StorageError};

const RETENTION_SECONDS: i64 = 30 * 24 * 60 * 60;
const MAX_INCIDENTS_PER_WORKSPACE: i64 = 200;
const MAX_BATCH_WORKSPACES: usize = 100;
const MAX_FUTURE_SKEW_SECONDS: i64 = 5 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeEventCategory {
    DiskPressure,
    Evicted,
    TemporaryStorageProvisioning,
    TemporaryStorageAttachment,
    VolumeUnavailable,
    Other,
}

impl RuntimeEventCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DiskPressure => "disk_pressure",
            Self::Evicted => "evicted",
            Self::TemporaryStorageProvisioning => "temporary_storage_provisioning",
            Self::TemporaryStorageAttachment => "temporary_storage_attachment",
            Self::VolumeUnavailable => "volume_unavailable",
            Self::Other => "other",
        }
    }

    fn from_stored(value: &str) -> Result<Self, StorageError> {
        match value {
            "disk_pressure" => Ok(Self::DiskPressure),
            "evicted" => Ok(Self::Evicted),
            "temporary_storage_provisioning" => Ok(Self::TemporaryStorageProvisioning),
            "temporary_storage_attachment" => Ok(Self::TemporaryStorageAttachment),
            "volume_unavailable" => Ok(Self::VolumeUnavailable),
            "other" => Ok(Self::Other),
            _ => Err(StorageError::UnknownRuntimeIncidentCategory(
                value.to_owned(),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewWorkspaceRuntimeIncident {
    pub category: RuntimeEventCategory,
    pub observed_at: i64,
    pub count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceRuntimeIncident {
    pub workspace_id: Uuid,
    pub category: RuntimeEventCategory,
    pub observed_at: i64,
    pub count: u32,
}

impl Database {
    /// Records a bounded set of normalized observations for one workspace.
    ///
    /// Kubernetes event counts are cumulative, so a repeated observation keeps
    /// the greatest count instead of adding it again. Raw event messages and
    /// node identities never enter this interface or the database.
    pub async fn upsert_workspace_runtime_incidents(
        &self,
        workspace_id: Uuid,
        incidents: &[NewWorkspaceRuntimeIncident],
        now: i64,
    ) -> Result<(), StorageError> {
        let incidents = normalized_incidents(incidents, now);
        let cutoff = retention_cutoff(now);
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                for incident in incidents {
                    sqlx::query("INSERT INTO workspace_runtime_incidents (id, installation_id, workspace_id, category, observed_at, count, first_seen_at, last_seen_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7) ON CONFLICT (installation_id, workspace_id, category, observed_at) DO UPDATE SET count = MAX(workspace_runtime_incidents.count, excluded.count), last_seen_at = MAX(workspace_runtime_incidents.last_seen_at, excluded.last_seen_at)")
                        .bind(Uuid::now_v7().to_string())
                        .bind(installation_id.as_str())
                        .bind(workspace_id.to_string())
                        .bind(incident.category.as_str())
                        .bind(incident.observed_at)
                        .bind(i64::from(incident.count))
                        .bind(now)
                        .execute(&mut *transaction)
                        .await?;
                }
                prune_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    workspace_id,
                    cutoff,
                )
                .await?;
                transaction.commit().await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                for incident in incidents {
                    sqlx::query("INSERT INTO workspace_runtime_incidents (id, installation_id, workspace_id, category, observed_at, count, first_seen_at, last_seen_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $7) ON CONFLICT (installation_id, workspace_id, category, observed_at) DO UPDATE SET count = GREATEST(workspace_runtime_incidents.count, EXCLUDED.count), last_seen_at = GREATEST(workspace_runtime_incidents.last_seen_at, EXCLUDED.last_seen_at)")
                        .bind(Uuid::now_v7().to_string())
                        .bind(installation_id.as_str())
                        .bind(workspace_id.to_string())
                        .bind(incident.category.as_str())
                        .bind(incident.observed_at)
                        .bind(i64::from(incident.count))
                        .bind(now)
                        .execute(&mut *transaction)
                        .await?;
                }
                prune_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    workspace_id,
                    cutoff,
                )
                .await?;
                transaction.commit().await?;
            }
        }
        Ok(())
    }

    /// Reads normalized incident history for a bounded workspace page.
    pub async fn list_workspace_runtime_incidents(
        &self,
        workspace_ids: &[Uuid],
        now: i64,
        limit_per_workspace: usize,
    ) -> Result<BTreeMap<Uuid, Vec<WorkspaceRuntimeIncident>>, StorageError> {
        if workspace_ids.is_empty() || limit_per_workspace == 0 {
            return Ok(BTreeMap::new());
        }
        if workspace_ids.len() > MAX_BATCH_WORKSPACES {
            return Err(StorageError::InvalidWorkspace);
        }
        let limit = limit_per_workspace.min(MAX_INCIDENTS_PER_WORKSPACE as usize);
        let cutoff = retention_cutoff(now);
        let incidents = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut query = QueryBuilder::<Sqlite>::new(
                    "SELECT workspace_id, category, observed_at, count FROM workspace_runtime_incidents WHERE installation_id = ",
                );
                query.push_bind(installation_id.as_str());
                query.push(" AND last_seen_at >= ");
                query.push_bind(cutoff);
                query.push(" AND workspace_id IN (");
                let mut separated = query.separated(", ");
                for workspace_id in workspace_ids {
                    separated.push_bind(workspace_id.to_string());
                }
                separated.push_unseparated(")");
                query.push(" ORDER BY workspace_id, observed_at DESC, category, id DESC");
                query
                    .build()
                    .fetch_all(pool)
                    .await?
                    .into_iter()
                    .map(decode_sqlite)
                    .collect::<Result<Vec<_>, _>>()?
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut query = QueryBuilder::<Postgres>::new(
                    "SELECT workspace_id, category, observed_at, count FROM workspace_runtime_incidents WHERE installation_id = ",
                );
                query.push_bind(installation_id.as_str());
                query.push(" AND last_seen_at >= ");
                query.push_bind(cutoff);
                query.push(" AND workspace_id IN (");
                let mut separated = query.separated(", ");
                for workspace_id in workspace_ids {
                    separated.push_bind(workspace_id.to_string());
                }
                separated.push_unseparated(")");
                query.push(" ORDER BY workspace_id, observed_at DESC, category, id DESC");
                query
                    .build()
                    .fetch_all(pool)
                    .await?
                    .into_iter()
                    .map(decode_postgres)
                    .collect::<Result<Vec<_>, _>>()?
            }
        };
        Ok(group_incidents(incidents, limit))
    }
}

async fn prune_sqlite(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    installation_id: &str,
    workspace_id: Uuid,
    cutoff: i64,
) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM workspace_runtime_incidents WHERE installation_id = ?1 AND workspace_id = ?2 AND last_seen_at < ?3")
        .bind(installation_id)
        .bind(workspace_id.to_string())
        .bind(cutoff)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("DELETE FROM workspace_runtime_incidents WHERE installation_id = ?1 AND workspace_id = ?2 AND id NOT IN (SELECT id FROM workspace_runtime_incidents WHERE installation_id = ?1 AND workspace_id = ?2 ORDER BY observed_at DESC, category, id DESC LIMIT ?3)")
        .bind(installation_id)
        .bind(workspace_id.to_string())
        .bind(MAX_INCIDENTS_PER_WORKSPACE)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn prune_postgres(
    transaction: &mut sqlx::Transaction<'_, Postgres>,
    installation_id: &str,
    workspace_id: Uuid,
    cutoff: i64,
) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM workspace_runtime_incidents WHERE installation_id = $1 AND workspace_id = $2 AND last_seen_at < $3")
        .bind(installation_id)
        .bind(workspace_id.to_string())
        .bind(cutoff)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("DELETE FROM workspace_runtime_incidents WHERE installation_id = $1 AND workspace_id = $2 AND id NOT IN (SELECT id FROM workspace_runtime_incidents WHERE installation_id = $1 AND workspace_id = $2 ORDER BY observed_at DESC, category, id DESC LIMIT $3)")
        .bind(installation_id)
        .bind(workspace_id.to_string())
        .bind(MAX_INCIDENTS_PER_WORKSPACE)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

fn normalized_incidents(
    incidents: &[NewWorkspaceRuntimeIncident],
    now: i64,
) -> Vec<NewWorkspaceRuntimeIncident> {
    let cutoff = retention_cutoff(now);
    let latest = now.saturating_add(MAX_FUTURE_SKEW_SECONDS);
    let mut normalized = BTreeMap::new();
    for incident in incidents {
        if incident.observed_at < cutoff || incident.observed_at > latest {
            continue;
        }
        normalized
            .entry((incident.category, incident.observed_at))
            .and_modify(|count: &mut u32| *count = (*count).max(incident.count.max(1)))
            .or_insert(incident.count.max(1));
    }
    normalized
        .into_iter()
        .map(
            |((category, observed_at), count)| NewWorkspaceRuntimeIncident {
                category,
                observed_at,
                count,
            },
        )
        .collect()
}

fn retention_cutoff(now: i64) -> i64 {
    now.saturating_sub(RETENTION_SECONDS)
}

fn decode_sqlite(row: SqliteRow) -> Result<WorkspaceRuntimeIncident, StorageError> {
    decode(row)
}

fn decode_postgres(row: PgRow) -> Result<WorkspaceRuntimeIncident, StorageError> {
    decode(row)
}

fn decode<R>(row: R) -> Result<WorkspaceRuntimeIncident, StorageError>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
    i64: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
{
    let count = row.try_get::<i64, _>("count")?;
    Ok(WorkspaceRuntimeIncident {
        workspace_id: Uuid::parse_str(&row.try_get::<String, _>("workspace_id")?)?,
        category: RuntimeEventCategory::from_stored(&row.try_get::<String, _>("category")?)?,
        observed_at: row.try_get("observed_at")?,
        count: u32::try_from(count).map_err(|_| StorageError::InvalidWorkspace)?,
    })
}

fn group_incidents(
    incidents: Vec<WorkspaceRuntimeIncident>,
    limit: usize,
) -> BTreeMap<Uuid, Vec<WorkspaceRuntimeIncident>> {
    let mut grouped = BTreeMap::<Uuid, Vec<WorkspaceRuntimeIncident>>::new();
    for incident in incidents {
        let workspace = grouped.entry(incident.workspace_id).or_default();
        if workspace.len() < limit {
            workspace.push(incident);
        }
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_deduplicates_counts_and_rejects_unbounded_timestamps() {
        let now = 2_000_000_000;
        let incidents = normalized_incidents(
            &[
                incident(RuntimeEventCategory::Evicted, now - 5, 1),
                incident(RuntimeEventCategory::Evicted, now - 5, 7),
                incident(RuntimeEventCategory::Other, now - RETENTION_SECONDS - 1, 2),
                incident(
                    RuntimeEventCategory::DiskPressure,
                    now + MAX_FUTURE_SKEW_SECONDS + 1,
                    3,
                ),
            ],
            now,
        );
        assert_eq!(
            incidents,
            vec![incident(RuntimeEventCategory::Evicted, now - 5, 7)]
        );
    }

    #[test]
    fn grouping_enforces_the_per_workspace_response_limit() {
        let first = Uuid::now_v7();
        let second = Uuid::now_v7();
        let grouped = group_incidents(
            vec![
                stored(first, 3),
                stored(first, 2),
                stored(first, 1),
                stored(second, 1),
            ],
            2,
        );
        assert_eq!(grouped[&first].len(), 2);
        assert_eq!(grouped[&second].len(), 1);
    }

    fn incident(
        category: RuntimeEventCategory,
        observed_at: i64,
        count: u32,
    ) -> NewWorkspaceRuntimeIncident {
        NewWorkspaceRuntimeIncident {
            category,
            observed_at,
            count,
        }
    }

    fn stored(workspace_id: Uuid, observed_at: i64) -> WorkspaceRuntimeIncident {
        WorkspaceRuntimeIncident {
            workspace_id,
            category: RuntimeEventCategory::Other,
            observed_at,
            count: 1,
        }
    }
}
