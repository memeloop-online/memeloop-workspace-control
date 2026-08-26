use sqlx::Row;
use uuid::Uuid;

use crate::injections::InjectionScope;

use super::{Database, InjectionScopeRef, NewJob, StorageError};

impl Database {
    /// Schedules every live workspace whose materialized cascade or SSH access
    /// set can be affected by a replaced injection.
    pub async fn enqueue_injection_reconciles(
        &self,
        scope: InjectionScopeRef,
        now: i64,
    ) -> Result<usize, StorageError> {
        let workspace_ids = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let sql = affected_sql(scope.scope, "?1", "?2");
                sqlx::query(&sql)
                    .bind(installation_id.as_str())
                    .bind(scope.scope_id.to_string())
                    .fetch_all(pool)
                    .await?
                    .into_iter()
                    .map(|row| -> Result<Uuid, StorageError> {
                        let id: String = row.try_get("id")?;
                        Ok(Uuid::parse_str(&id)?)
                    })
                    .collect::<Result<Vec<_>, StorageError>>()?
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let sql = affected_sql(scope.scope, "$1", "$2");
                sqlx::query(&sql)
                    .bind(installation_id.as_str())
                    .bind(scope.scope_id.to_string())
                    .fetch_all(pool)
                    .await?
                    .into_iter()
                    .map(|row| -> Result<Uuid, StorageError> {
                        let id: String = row.try_get("id")?;
                        Ok(Uuid::parse_str(&id)?)
                    })
                    .collect::<Result<Vec<_>, StorageError>>()?
            }
        };
        for workspace_id in &workspace_ids {
            self.enqueue_job(
                NewJob {
                    kind: "reconcile_workspace".to_owned(),
                    workspace_id: Some(*workspace_id),
                    payload: serde_json::json!({"reason": "injection_changed"}),
                    available_at: now,
                },
                now,
            )
            .await?;
        }
        Ok(workspace_ids.len())
    }
}

fn affected_sql(scope: InjectionScope, installation: &str, scope_id: &str) -> String {
    let predicate = match scope {
        InjectionScope::Organization => format!("w.organization_id = {scope_id}"),
        InjectionScope::Workspace => format!("w.id = {scope_id}"),
        InjectionScope::User => format!(
            "(w.owner_id = {scope_id} OR EXISTS (SELECT 1 FROM organization_memberships m \
             WHERE m.installation_id = w.installation_id AND m.organization_id = \
             w.organization_id AND m.user_id = {scope_id}) OR EXISTS (SELECT 1 FROM users u \
             WHERE u.installation_id = w.installation_id AND u.id = {scope_id} AND \
             u.system_admin <> 0 AND u.disabled = 0))"
        ),
    };
    format!(
        "SELECT DISTINCT w.id FROM workspaces w WHERE w.installation_id = {installation} AND \
         w.state NOT IN ('deleting', 'deleted') AND {predicate} ORDER BY w.id"
    )
}
