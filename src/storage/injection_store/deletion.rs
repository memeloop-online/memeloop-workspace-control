use sqlx::{PgConnection, Row, SqliteConnection};
use uuid::Uuid;

use super::{Database, InjectionScope, InjectionScopeRef, StorageError};

impl Database {
    pub async fn delete_injection(
        &self,
        scope: InjectionScopeRef,
        key: &str,
        allow_locked: bool,
        actor: Uuid,
        now: i64,
    ) -> Result<bool, StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                let deleted = delete_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    scope,
                    key,
                    allow_locked,
                    actor,
                    now,
                )
                .await?;
                transaction.commit().await?;
                Ok(deleted)
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                let deleted = delete_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    scope,
                    key,
                    allow_locked,
                    actor,
                    now,
                )
                .await?;
                transaction.commit().await?;
                Ok(deleted)
            }
        }
    }
}

async fn delete_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    scope: InjectionScopeRef,
    key: &str,
    allow_locked: bool,
    actor: Uuid,
    now: i64,
) -> Result<bool, StorageError> {
    let row = sqlx::query("SELECT locked, version FROM injection_items WHERE installation_id = ?1 AND scope = ?2 AND scope_id = ?3 AND key = ?4")
        .bind(installation)
        .bind(scope.scope.as_str())
        .bind(scope.scope_id.to_string())
        .bind(key)
        .fetch_optional(&mut *connection)
        .await?;
    let Some(row) = row else {
        return Ok(false);
    };
    ensure_unlocked(row.try_get::<i64, _>("locked")? != 0, allow_locked)?;
    let version: i64 = row.try_get("version")?;
    sqlx::query("DELETE FROM injection_items WHERE installation_id = ?1 AND scope = ?2 AND scope_id = ?3 AND key = ?4")
        .bind(installation)
        .bind(scope.scope.as_str())
        .bind(scope.scope_id.to_string())
        .bind(key)
        .execute(&mut *connection)
        .await?;
    audit_sqlite(connection, installation, scope, key, version, actor, now).await?;
    Ok(true)
}

async fn delete_postgres(
    connection: &mut PgConnection,
    installation: &str,
    scope: InjectionScopeRef,
    key: &str,
    allow_locked: bool,
    actor: Uuid,
    now: i64,
) -> Result<bool, StorageError> {
    let row = sqlx::query("SELECT locked, version FROM injection_items WHERE installation_id = $1 AND scope = $2 AND scope_id = $3 AND key = $4 FOR UPDATE")
        .bind(installation)
        .bind(scope.scope.as_str())
        .bind(scope.scope_id.to_string())
        .bind(key)
        .fetch_optional(&mut *connection)
        .await?;
    let Some(row) = row else {
        return Ok(false);
    };
    ensure_unlocked(row.try_get::<i64, _>("locked")? != 0, allow_locked)?;
    let version: i64 = row.try_get("version")?;
    sqlx::query("DELETE FROM injection_items WHERE installation_id = $1 AND scope = $2 AND scope_id = $3 AND key = $4")
        .bind(installation)
        .bind(scope.scope.as_str())
        .bind(scope.scope_id.to_string())
        .bind(key)
        .execute(&mut *connection)
        .await?;
    audit_postgres(connection, installation, scope, key, version, actor, now).await?;
    Ok(true)
}

fn ensure_unlocked(locked: bool, allow_locked: bool) -> Result<(), StorageError> {
    if locked && !allow_locked {
        return Err(StorageError::InvalidInjectionLock);
    }
    Ok(())
}

async fn audit_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    scope: InjectionScopeRef,
    key: &str,
    version: i64,
    actor: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, 'injection.delete', ?6, ?7)")
        .bind(Uuid::now_v7().to_string())
        .bind(installation)
        .bind(actor.to_string())
        .bind(organization_id(scope))
        .bind(workspace_id(scope))
        .bind(metadata(scope, key, version))
        .bind(now)
        .execute(&mut *connection)
        .await?;
    Ok(())
}

async fn audit_postgres(
    connection: &mut PgConnection,
    installation: &str,
    scope: InjectionScopeRef,
    key: &str,
    version: i64,
    actor: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES ($1, $2, $3, $4, $5, 'injection.delete', $6, $7)")
        .bind(Uuid::now_v7().to_string())
        .bind(installation)
        .bind(actor.to_string())
        .bind(organization_id(scope))
        .bind(workspace_id(scope))
        .bind(metadata(scope, key, version))
        .bind(now)
        .execute(&mut *connection)
        .await?;
    Ok(())
}

fn organization_id(scope: InjectionScopeRef) -> Option<String> {
    (scope.scope == InjectionScope::Organization).then(|| scope.scope_id.to_string())
}

fn workspace_id(scope: InjectionScopeRef) -> Option<String> {
    (scope.scope == InjectionScope::Workspace).then(|| scope.scope_id.to_string())
}

fn metadata(scope: InjectionScopeRef, key: &str, version: i64) -> String {
    serde_json::json!({
        "key": key,
        "scope": scope.scope.as_str(),
        "version": version,
    })
    .to_string()
}
