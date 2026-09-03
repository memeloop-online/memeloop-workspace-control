use sqlx::{PgConnection, Row, SqliteConnection};
use uuid::Uuid;

use super::super::idempotency::{self, IdempotencyCompletion};
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
        let keys = [key.to_owned()];
        Ok(self
            .delete_injections(scope, &keys, allow_locked, actor, now)
            .await?
            == 1)
    }

    /// Deletes one logical batch in a single database transaction. Audit rows are retained for
    /// every item that existed, while missing keys remain idempotent successes.
    pub async fn delete_injections(
        &self,
        scope: InjectionScopeRef,
        keys: &[String],
        allow_locked: bool,
        actor: Uuid,
        now: i64,
    ) -> Result<usize, StorageError> {
        let keys = sorted_unique_keys(keys);
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                let deleted = delete_many_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    scope,
                    &keys,
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
                let deleted = delete_many_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    scope,
                    &keys,
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

    /// Atomically commits the deletion audit tombstones and one reconciliation job per affected
    /// workspace. A failed transaction can never leave Kubernetes materialization stale.
    pub async fn delete_injections_and_enqueue_reconciles(
        &self,
        scope: InjectionScopeRef,
        keys: &[String],
        allow_locked: bool,
        actor: Uuid,
        now: i64,
        completion: IdempotencyCompletion<'_>,
    ) -> Result<usize, StorageError> {
        let keys = sorted_unique_keys(keys);
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                let deleted = delete_many_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    scope,
                    &keys,
                    allow_locked,
                    actor,
                    now,
                )
                .await?;
                if deleted > 0 {
                    super::super::injection_reconcile::enqueue_sqlite(
                        &mut transaction,
                        installation_id.as_str(),
                        scope,
                        now,
                    )
                    .await?;
                }
                let finished = idempotency::finish_sqlite(
                    &mut *transaction,
                    installation_id.as_str(),
                    completion,
                )
                .await?;
                idempotency::ensure_finished(finished)?;
                transaction.commit().await?;
                Ok(deleted)
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                let deleted = delete_many_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    scope,
                    &keys,
                    allow_locked,
                    actor,
                    now,
                )
                .await?;
                if deleted > 0 {
                    super::super::injection_reconcile::enqueue_postgres(
                        &mut transaction,
                        installation_id.as_str(),
                        scope,
                        now,
                    )
                    .await?;
                }
                let finished = idempotency::finish_postgres(
                    &mut *transaction,
                    installation_id.as_str(),
                    completion,
                )
                .await?;
                idempotency::ensure_finished(finished)?;
                transaction.commit().await?;
                Ok(deleted)
            }
        }
    }
}

fn sorted_unique_keys(keys: &[String]) -> Vec<&str> {
    let mut keys = keys.iter().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    keys.dedup();
    keys
}

async fn delete_many_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    scope: InjectionScopeRef,
    keys: &[&str],
    allow_locked: bool,
    actor: Uuid,
    now: i64,
) -> Result<usize, StorageError> {
    let mut deleted = 0;
    for key in keys {
        deleted += usize::from(
            delete_sqlite(
                connection,
                installation,
                scope,
                key,
                allow_locked,
                actor,
                now,
            )
            .await?,
        );
    }
    Ok(deleted)
}

async fn delete_many_postgres(
    connection: &mut PgConnection,
    installation: &str,
    scope: InjectionScopeRef,
    keys: &[&str],
    allow_locked: bool,
    actor: Uuid,
    now: i64,
) -> Result<usize, StorageError> {
    let mut deleted = 0;
    for key in keys {
        deleted += usize::from(
            delete_postgres(
                connection,
                installation,
                scope,
                key,
                allow_locked,
                actor,
                now,
            )
            .await?,
        );
    }
    Ok(deleted)
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
