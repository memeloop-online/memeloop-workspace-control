use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sqlx::{Postgres, QueryBuilder, Row, Sqlite, postgres::PgRow, sqlite::SqliteRow};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{Database, StorageError};
use crate::runtime_incidents::RuntimeEventDetails;

const RETENTION_SECONDS: i64 = 30 * 24 * 60 * 60;
const MAX_INCIDENTS_PER_WORKSPACE: i64 = 200;
const MAX_BATCH_WORKSPACES: usize = 100;
const MAX_FUTURE_SKEW_SECONDS: i64 = 5 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeEventCategory {
    EphemeralStorage,
    MemoryPressure,
    PidPressure,
    DiskPressure,
    Evicted,
    TemporaryStorageProvisioning,
    TemporaryStorageAttachment,
    VolumeUnavailable,
    ContainerBackoff,
    ImagePull,
    Scheduling,
    HealthCheck,
    Other,
}

impl RuntimeEventCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EphemeralStorage => "ephemeral_storage",
            Self::MemoryPressure => "memory_pressure",
            Self::PidPressure => "pid_pressure",
            Self::DiskPressure => "disk_pressure",
            Self::Evicted => "evicted",
            Self::TemporaryStorageProvisioning => "temporary_storage_provisioning",
            Self::TemporaryStorageAttachment => "temporary_storage_attachment",
            Self::VolumeUnavailable => "volume_unavailable",
            Self::ContainerBackoff => "container_backoff",
            Self::ImagePull => "image_pull",
            Self::Scheduling => "scheduling",
            Self::HealthCheck => "health_check",
            Self::Other => "other",
        }
    }

    fn from_stored(value: &str) -> Result<Self, StorageError> {
        match value {
            "ephemeral_storage" => Ok(Self::EphemeralStorage),
            "memory_pressure" => Ok(Self::MemoryPressure),
            "pid_pressure" => Ok(Self::PidPressure),
            "disk_pressure" => Ok(Self::DiskPressure),
            "evicted" => Ok(Self::Evicted),
            "temporary_storage_provisioning" => Ok(Self::TemporaryStorageProvisioning),
            "temporary_storage_attachment" => Ok(Self::TemporaryStorageAttachment),
            "volume_unavailable" => Ok(Self::VolumeUnavailable),
            "container_backoff" => Ok(Self::ContainerBackoff),
            "image_pull" => Ok(Self::ImagePull),
            "scheduling" => Ok(Self::Scheduling),
            "health_check" => Ok(Self::HealthCheck),
            "other" => Ok(Self::Other),
            _ => Err(StorageError::UnknownRuntimeIncidentCategory(
                value.to_owned(),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewWorkspaceRuntimeIncident {
    pub id: String,
    pub details: RuntimeEventDetails,
    pub category: RuntimeEventCategory,
    /// Stable beginning of the Kubernetes Event, not its latest recurrence.
    pub observed_at: i64,
    /// Latest recurrence, used for retention without changing the incident key.
    pub last_observed_at: i64,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRuntimeIncident {
    pub id: String,
    pub details: RuntimeEventDetails,
    pub last_observed_at: i64,
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
                    sqlx::query("INSERT INTO workspace_runtime_incidents (id, installation_id, workspace_id, category, observed_at, count, first_seen_at, last_seen_at, details_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?5, ?7, ?8) ON CONFLICT (id) DO UPDATE SET count = MAX(workspace_runtime_incidents.count, excluded.count), details_json = CASE WHEN excluded.last_seen_at >= workspace_runtime_incidents.last_seen_at THEN excluded.details_json ELSE workspace_runtime_incidents.details_json END, last_seen_at = MAX(workspace_runtime_incidents.last_seen_at, excluded.last_seen_at)")
                        .bind(&incident.id)
                        .bind(installation_id.as_str())
                        .bind(workspace_id.to_string())
                        .bind(incident.category.as_str())
                        .bind(incident.observed_at)
                        .bind(i64::from(incident.count))
                        .bind(incident.last_observed_at)
                        .bind(serde_json::to_string(&incident.details)?)
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
                    sqlx::query("INSERT INTO workspace_runtime_incidents (id, installation_id, workspace_id, category, observed_at, count, first_seen_at, last_seen_at, details_json) VALUES ($1, $2, $3, $4, $5, $6, $5, $7, $8) ON CONFLICT (id) DO UPDATE SET count = GREATEST(workspace_runtime_incidents.count, EXCLUDED.count), details_json = CASE WHEN EXCLUDED.last_seen_at >= workspace_runtime_incidents.last_seen_at THEN EXCLUDED.details_json ELSE workspace_runtime_incidents.details_json END, last_seen_at = GREATEST(workspace_runtime_incidents.last_seen_at, EXCLUDED.last_seen_at)")
                        .bind(&incident.id)
                        .bind(installation_id.as_str())
                        .bind(workspace_id.to_string())
                        .bind(incident.category.as_str())
                        .bind(incident.observed_at)
                        .bind(i64::from(incident.count))
                        .bind(incident.last_observed_at)
                        .bind(serde_json::to_string(&incident.details)?)
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
                    "SELECT id, details_json, workspace_id, category, observed_at, last_seen_at, count FROM workspace_runtime_incidents WHERE installation_id = ",
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
                    "SELECT id, details_json, workspace_id, category, observed_at, last_seen_at, count FROM workspace_runtime_incidents WHERE installation_id = ",
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
        if incident.last_observed_at < cutoff
            || incident.last_observed_at > latest
            || incident.observed_at > incident.last_observed_at
        {
            continue;
        }
        normalized
            .entry(incident.id.clone())
            .and_modify(|entry: &mut NewWorkspaceRuntimeIncident| {
                entry.count = entry.count.max(incident.count.max(1));
                if incident.last_observed_at >= entry.last_observed_at {
                    entry.details = incident.details.clone();
                    entry.last_observed_at = incident.last_observed_at;
                }
            })
            .or_insert_with(|| {
                let mut incident = incident.clone();
                incident.count = incident.count.max(1);
                incident
            });
    }
    normalized.into_values().collect()
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
        id: row.try_get("id")?,
        details: serde_json::from_str(&row.try_get::<String, _>("details_json")?)?,
        last_observed_at: row.try_get("last_seen_at")?,
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
            id: format!("{}:{observed_at}", category.as_str()),
            details: RuntimeEventDetails::default(),
            category,
            observed_at,
            last_observed_at: observed_at,
            count,
        }
    }

    fn stored(workspace_id: Uuid, observed_at: i64) -> WorkspaceRuntimeIncident {
        WorkspaceRuntimeIncident {
            id: format!("{workspace_id}:{observed_at}"),
            details: RuntimeEventDetails::default(),
            last_observed_at: observed_at,
            workspace_id,
            category: RuntimeEventCategory::Other,
            observed_at,
            count: 1,
        }
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    use crate::runtime_incidents::{RuntimeContainer, RuntimeDiagnostic, SchedulingBlocker};

    async fn assert_history(database: &Database) {
        let table = super::super::schema::BASELINE
            .iter()
            .find(|statement| statement.starts_with("CREATE TABLE workspace_runtime_incidents "))
            .unwrap();
        match database {
            Database::Sqlite { pool, .. } => {
                sqlx::query("CREATE TABLE workspaces (id TEXT PRIMARY KEY)")
                    .execute(pool)
                    .await
                    .unwrap();
                sqlx::query(table).execute(pool).await.unwrap();
            }
            Database::Postgres { pool, .. } => {
                sqlx::query("CREATE TABLE workspaces (id TEXT PRIMARY KEY)")
                    .execute(pool)
                    .await
                    .unwrap();
                sqlx::query(table).execute(pool).await.unwrap();
            }
        }
        let workspace_id = Uuid::now_v7();
        let parent = format!("INSERT INTO workspaces (id) VALUES ('{workspace_id}')");
        match database {
            Database::Sqlite { pool, .. } => {
                sqlx::query(&parent).execute(pool).await.unwrap();
            }
            Database::Postgres { pool, .. } => {
                sqlx::query(&parent).execute(pool).await.unwrap();
            }
        }
        let now = 2_000_000_000;
        let first_uid = Uuid::now_v7();
        let first = NewWorkspaceRuntimeIncident {
            id: first_uid.to_string(),
            category: RuntimeEventCategory::EphemeralStorage,
            observed_at: now - 10,
            last_observed_at: now - 10,
            count: 2,
            details: RuntimeEventDetails {
                event_uid: Some(first_uid),
                pod_uid: Some(Uuid::now_v7()),
                reason: Some("Evicted".into()),
                container: Some(RuntimeContainer::Workspace),
                diagnostic: Some(RuntimeDiagnostic::ContainerLocalEphemeralLimit {
                    container: RuntimeContainer::Workspace,
                    limit_bytes: 2 * 1024 * 1024 * 1024,
                }),
                ..Default::default()
            },
        };
        let second_uid = Uuid::now_v7();
        let mut second = first.clone();
        second.id = second_uid.to_string();
        second.details.event_uid = Some(second_uid);
        second.details.pod_uid = Some(Uuid::now_v7());
        second.category = RuntimeEventCategory::Scheduling;
        second.details.reason = Some("FailedScheduling".into());
        second.details.container = None;
        second.details.diagnostic = None;
        second.details.scheduling_blockers = vec![SchedulingBlocker::StorageCapacity];
        let mut recurrence = first.clone();
        recurrence.count = 7;
        recurrence.last_observed_at = now;
        database
            .upsert_workspace_runtime_incidents(
                workspace_id,
                &[
                    first.clone(),
                    second.clone(),
                    recurrence.clone(),
                    first.clone(),
                ],
                now,
            )
            .await
            .unwrap();
        let mut latest_second = second.clone();
        latest_second.last_observed_at = now;
        latest_second.count = 5;
        latest_second.details.scheduling_blockers = vec![SchedulingBlocker::CpuCapacity];
        database
            .upsert_workspace_runtime_incidents(
                workspace_id,
                &[second.clone(), latest_second.clone(), second.clone()],
                now,
            )
            .await
            .unwrap();
        // A later scan can return an older observation; it must not roll back facts.
        database
            .upsert_workspace_runtime_incidents(workspace_id, &[second.clone()], now)
            .await
            .unwrap();
        // Kubernetes Events have expired, but the stored facts do not depend on
        // a live Event or on the replacement Pod's identity/resource limits.
        let history = database
            .list_workspace_runtime_incidents(&[workspace_id], now + 7200, 50)
            .await
            .unwrap();
        assert_eq!(history[&workspace_id].len(), 2);
        let restored = history[&workspace_id]
            .iter()
            .find(|row| row.id == first.id)
            .unwrap();
        assert_eq!(restored.details, first.details);
        assert_eq!(restored.count, 7);
        assert_eq!(restored.last_observed_at, now);
        let restored_second = history[&workspace_id]
            .iter()
            .find(|row| row.id == second.id)
            .unwrap();
        assert_eq!(restored_second.details, latest_second.details);
        assert_eq!(restored_second.count, 5);
        assert_eq!(restored_second.observed_at, second.observed_at);

        let mut recent_recurrence = recurrence;
        recent_recurrence.observed_at = now - RETENTION_SECONDS - 100;
        let recurrence_uid = Uuid::now_v7();
        recent_recurrence.id = recurrence_uid.to_string();
        recent_recurrence.details.event_uid = Some(recurrence_uid);
        database
            .upsert_workspace_runtime_incidents(workspace_id, &[recent_recurrence], now)
            .await
            .unwrap();
        assert_eq!(
            database
                .list_workspace_runtime_incidents(&[workspace_id], now, 50)
                .await
                .unwrap()[&workspace_id]
                .len(),
            3
        );

        let newer = (0..205)
            .map(|offset| {
                let uid = Uuid::now_v7();
                let mut incident = first.clone();
                incident.id = uid.to_string();
                incident.details.event_uid = Some(uid);
                incident.observed_at = now + offset;
                incident.last_observed_at = now + offset;
                incident
            })
            .collect::<Vec<_>>();
        database
            .upsert_workspace_runtime_incidents(workspace_id, &newer, now + 205)
            .await
            .unwrap();
        let bounded = database
            .list_workspace_runtime_incidents(&[workspace_id], now + 205, 500)
            .await
            .unwrap();
        assert_eq!(
            bounded[&workspace_id].len(),
            MAX_INCIDENTS_PER_WORKSPACE as usize
        );
        assert_eq!(bounded[&workspace_id].last().unwrap().observed_at, now + 5);
        database
            .upsert_workspace_runtime_incidents(workspace_id, &[], now + 205 + RETENTION_SECONDS)
            .await
            .unwrap();
        assert!(
            database
                .list_workspace_runtime_incidents(
                    &[workspace_id],
                    now + 205 + RETENTION_SECONDS,
                    500
                )
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn sqlite_preserves_typed_details_source_identity_counts_and_retention() {
        let database = Database::connect("sqlite::memory:", "history-sqlite".parse().unwrap())
            .await
            .unwrap();
        assert_history(&database).await;
    }

    #[tokio::test]
    async fn postgres_preserves_typed_details_source_identity_counts_and_retention_when_configured()
    {
        let Ok(url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
            return;
        };
        let admin = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        let schema = format!("mwc_event_history_{}", Uuid::now_v7().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        let mut scoped = url::Url::parse(&url).unwrap();
        scoped
            .query_pairs_mut()
            .append_pair("options", &format!("-c search_path={schema}"));
        let database = Database::connect(scoped.as_str(), "history-pg".parse().unwrap())
            .await
            .unwrap();
        assert_history(&database).await;
        if let Database::Postgres { pool, .. } = &database {
            pool.close().await;
        }
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin)
            .await
            .unwrap();
        admin.close().await;
    }
}
