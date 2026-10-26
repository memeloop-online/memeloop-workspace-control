//! A watch can request a read-only reconciliation, never change workspace intent.
use sqlx::Row;
use uuid::Uuid;

use super::{Database, StorageError};

impl Database {
    /// Coalesce Pod and StatefulSet events with any reconciliation already queued.
    /// The generation is read from the same transaction as the conditional insert.
    pub(crate) async fn enqueue_ready_workspace_observation(
        &self,
        workspace_id: Uuid,
        now: i64,
    ) -> Result<bool, StorageError> {
        let id = Uuid::now_v7().to_string();
        let workspace_id = workspace_id.to_string();
        let insert = "INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) ";
        let active = "NOT EXISTS (SELECT 1 FROM jobs j WHERE j.installation_id = w.installation_id AND j.workspace_id = w.id AND j.kind = 'reconcile_workspace' AND j.status IN ('pending', 'running'))";
        let affected = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                // One write statement serializes concurrent event producers in SQLite.
                sqlx::query(&format!("{insert} SELECT ?1, w.installation_id, 'reconcile_workspace', w.id, json_object('generation', w.generation, 'runtime_observation', json('true')), 'pending', ?4, NULL, NULL, 0, ?4, ?4 FROM workspaces w WHERE w.installation_id = ?2 AND w.id = ?3 AND w.state = 'ready' AND {active}"))
                    .bind(&id).bind(installation_id.as_str()).bind(&workspace_id).bind(now)
                    .execute(pool).await?.rows_affected()
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut tx = pool.begin().await?;
                // Separate statements ensure a waiting producer sees the first
                // producer's committed job after acquiring the workspace row lock.
                sqlx::query(
                    "SELECT id FROM workspaces WHERE installation_id = $1 AND id = $2 FOR UPDATE",
                )
                .bind(installation_id.as_str())
                .bind(&workspace_id)
                .fetch_optional(&mut *tx)
                .await?;
                let affected = sqlx::query(&format!("{insert} SELECT $1, w.installation_id, 'reconcile_workspace', w.id, json_build_object('generation', w.generation, 'runtime_observation', true)::text, 'pending', $4, NULL, NULL, 0, $4, $4 FROM workspaces w WHERE w.installation_id = $2 AND w.id = $3 AND w.state = 'ready' AND {active}"))
                    .bind(&id).bind(installation_id.as_str()).bind(&workspace_id).bind(now)
                    .execute(&mut *tx).await?.rows_affected();
                tx.commit().await?;
                affected
            }
        };
        Ok(affected == 1)
    }

    /// Used on a completed Kubernetes relist, including Ready rows whose Pod and
    /// StatefulSet have both disappeared while this controller was offline.
    pub(crate) async fn ready_workspace_ids(
        &self,
        after: Option<Uuid>,
    ) -> Result<Vec<Uuid>, StorageError> {
        let cursor = after.map(|id| id.to_string()).unwrap_or_default();
        let sql = "SELECT id FROM workspaces WHERE installation_id = $1 AND state = 'ready' AND id > $2 ORDER BY id LIMIT 256";
        let rows = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query(sql)
                    .bind(installation_id.as_str())
                    .bind(&cursor)
                    .fetch_all(pool)
                    .await?
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                return sqlx::query(sql)
                    .bind(installation_id.as_str())
                    .bind(&cursor)
                    .fetch_all(pool)
                    .await?
                    .into_iter()
                    .map(|row| Ok(Uuid::parse_str(row.try_get("id")?)?))
                    .collect();
            }
        };
        rows.into_iter()
            .map(|row| Ok(Uuid::parse_str(row.try_get("id")?)?))
            .collect()
    }
}
