use sqlx::{PgConnection, Row, SqliteConnection};
use uuid::Uuid;

use crate::injections::InjectionScope;

use super::{Database, InjectionScopeRef, StorageError};

impl Database {
    /// Schedules every live workspace whose materialized cascade or SSH access set can be affected
    /// by a changed injection. The complete set of jobs is committed atomically.
    pub async fn enqueue_injection_reconciles(
        &self,
        scope: InjectionScopeRef,
        now: i64,
    ) -> Result<usize, StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                let count =
                    enqueue_sqlite(&mut transaction, installation_id.as_str(), scope, now).await?;
                transaction.commit().await?;
                Ok(count)
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                let count =
                    enqueue_postgres(&mut transaction, installation_id.as_str(), scope, now)
                        .await?;
                transaction.commit().await?;
                Ok(count)
            }
        }
    }
}

pub(in crate::storage) async fn enqueue_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    scope: InjectionScopeRef,
    now: i64,
) -> Result<usize, StorageError> {
    let workspace_ids = affected_sqlite(connection, installation, scope).await?;
    for workspace_id in &workspace_ids {
        sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES (?1, ?2, 'reconcile_workspace', ?3, ?4, 'pending', ?5, NULL, NULL, 0, ?5, ?5)")
            .bind(Uuid::now_v7().to_string())
            .bind(installation)
            .bind(workspace_id.to_string())
            .bind(reconcile_payload())
            .bind(now)
            .execute(&mut *connection)
            .await?;
    }
    Ok(workspace_ids.len())
}

pub(in crate::storage) async fn enqueue_postgres(
    connection: &mut PgConnection,
    installation: &str,
    scope: InjectionScopeRef,
    now: i64,
) -> Result<usize, StorageError> {
    let workspace_ids = affected_postgres(connection, installation, scope).await?;
    for workspace_id in &workspace_ids {
        sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES ($1, $2, 'reconcile_workspace', $3, $4, 'pending', $5, NULL, NULL, 0, $5, $5)")
            .bind(Uuid::now_v7().to_string())
            .bind(installation)
            .bind(workspace_id.to_string())
            .bind(reconcile_payload())
            .bind(now)
            .execute(&mut *connection)
            .await?;
    }
    Ok(workspace_ids.len())
}

async fn affected_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    scope: InjectionScopeRef,
) -> Result<Vec<Uuid>, StorageError> {
    let sql = affected_sql(scope.scope, "?1", "?2");
    decode_workspace_ids(
        sqlx::query(&sql)
            .bind(installation)
            .bind(scope.scope_id.to_string())
            .fetch_all(connection)
            .await?,
    )
}

async fn affected_postgres(
    connection: &mut PgConnection,
    installation: &str,
    scope: InjectionScopeRef,
) -> Result<Vec<Uuid>, StorageError> {
    let sql = affected_sql(scope.scope, "$1", "$2");
    decode_workspace_ids(
        sqlx::query(&sql)
            .bind(installation)
            .bind(scope.scope_id.to_string())
            .fetch_all(connection)
            .await?,
    )
}

fn decode_workspace_ids<R: Row>(rows: Vec<R>) -> Result<Vec<Uuid>, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    rows.into_iter()
        .map(|row| -> Result<Uuid, StorageError> {
            let id: String = row.try_get("id")?;
            Ok(Uuid::parse_str(&id)?)
        })
        .collect()
}

fn reconcile_payload() -> String {
    serde_json::json!({"reason": "injection_changed"}).to_string()
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
