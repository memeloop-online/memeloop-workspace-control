use crate::config::InstallationId;
use sqlx::{PgPool, SqlitePool};

use super::{Database, StorageError, schema};

pub(super) async fn migrate(database: &Database) -> Result<(), StorageError> {
    let applied_at = unix_timestamp()?;
    match database {
        Database::Sqlite { pool, .. } => migrate_sqlite(pool, applied_at).await?,
        Database::Postgres {
            pool,
            installation_id,
        } => migrate_postgres(pool, installation_id, applied_at).await?,
    }
    database.ensure_installation_identity().await
}

async fn migrate_sqlite(pool: &SqlitePool, applied_at: i64) -> Result<(), StorageError> {
    let mut transaction = pool.begin().await?;
    let has_application_tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' \
         AND name NOT IN ('schema_migrations', 'sqlite_sequence')",
    )
    .fetch_one(&mut *transaction)
    .await?;
    sqlx::query(schema::MIGRATION_TABLE)
        .execute(&mut *transaction)
        .await?;
    let version = current_sqlite_version(&mut transaction).await?;
    match version {
        schema::SCHEMA_VERSION | 28 | 27 => {}
        24 => {
            upgrade_sqlite_v24_to_v25(&mut transaction, applied_at).await?;
            upgrade_sqlite_home_pvc(&mut transaction, applied_at).await?;
            upgrade_sqlite_runtime_categories(&mut transaction, applied_at).await?;
        }
        25 => {
            upgrade_sqlite_home_pvc(&mut transaction, applied_at).await?;
            upgrade_sqlite_runtime_categories(&mut transaction, applied_at).await?;
        }
        26 => upgrade_sqlite_runtime_categories(&mut transaction, applied_at).await?,
        0 if has_application_tables == 0 => {
            for statement in schema::BASELINE
                .iter()
                .chain(schema::HOME_PVC_MIGRATION)
                .chain(schema::STORAGE_SHRINK_MIGRATION)
                .chain(schema::RUNTIME_SETTINGS_MIGRATION)
            {
                sqlx::query(statement).execute(&mut *transaction).await?;
            }
            sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)")
                .bind(schema::SCHEMA_VERSION)
                .bind(applied_at)
                .execute(&mut *transaction)
                .await?;
        }
        _ => return Err(StorageError::UnsupportedDatabaseVersion),
    }
    if version != 0 && version < schema::SCHEMA_VERSION {
        if version < 28 {
            for statement in schema::STORAGE_SHRINK_MIGRATION {
                sqlx::query(statement).execute(&mut *transaction).await?;
            }
        }
        for statement in schema::RUNTIME_SETTINGS_MIGRATION {
            sqlx::query(statement).execute(&mut *transaction).await?;
        }
        sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)")
            .bind(schema::SCHEMA_VERSION)
            .bind(applied_at)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    Ok(())
}

async fn migrate_postgres(
    pool: &PgPool,
    installation_id: &InstallationId,
    applied_at: i64,
) -> Result<(), StorageError> {
    let mut transaction = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("mwc:migrate:{installation_id}"))
        .execute(&mut *transaction)
        .await?;
    let has_application_tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.tables \
         WHERE table_schema = current_schema() \
         AND table_name <> 'schema_migrations'",
    )
    .fetch_one(&mut *transaction)
    .await?;
    sqlx::query(schema::MIGRATION_TABLE)
        .execute(&mut *transaction)
        .await?;
    let version: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM schema_migrations")
            .fetch_one(&mut *transaction)
            .await?;
    match version {
        schema::SCHEMA_VERSION | 28 | 27 => {}
        24 => {
            upgrade_postgres_v24_to_v25(&mut transaction, applied_at).await?;
            upgrade_postgres_home_pvc(&mut transaction, applied_at).await?;
            upgrade_postgres_runtime_categories(&mut transaction, applied_at).await?;
        }
        25 => {
            upgrade_postgres_home_pvc(&mut transaction, applied_at).await?;
            upgrade_postgres_runtime_categories(&mut transaction, applied_at).await?;
        }
        26 => upgrade_postgres_runtime_categories(&mut transaction, applied_at).await?,
        0 if has_application_tables == 0 => {
            for statement in schema::BASELINE
                .iter()
                .chain(schema::HOME_PVC_MIGRATION)
                .chain(schema::STORAGE_SHRINK_MIGRATION)
                .chain(schema::RUNTIME_SETTINGS_MIGRATION)
            {
                sqlx::query(statement).execute(&mut *transaction).await?;
            }
            sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES ($1, $2)")
                .bind(schema::SCHEMA_VERSION)
                .bind(applied_at)
                .execute(&mut *transaction)
                .await?;
        }
        _ => return Err(StorageError::UnsupportedDatabaseVersion),
    }
    if version != 0 && version < schema::SCHEMA_VERSION {
        if version < 28 {
            for statement in schema::STORAGE_SHRINK_MIGRATION {
                sqlx::query(statement).execute(&mut *transaction).await?;
            }
        }
        for statement in schema::RUNTIME_SETTINGS_MIGRATION {
            sqlx::query(statement).execute(&mut *transaction).await?;
        }
        sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES ($1, $2)")
            .bind(schema::SCHEMA_VERSION)
            .bind(applied_at)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    Ok(())
}

/// Version 24 stored only a non-reversible token hash.  Existing keys remain
/// usable through that hash, but cannot be made copyable retroactively.
async fn upgrade_sqlite_v24_to_v25(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    applied_at: i64,
) -> Result<(), StorageError> {
    sqlx::query("ALTER TABLE user_api_keys ADD COLUMN token TEXT")
        .execute(&mut **transaction)
        .await?;
    sqlx::query("CREATE UNIQUE INDEX user_api_keys_token_unique_idx ON user_api_keys (installation_id, token)")
        .execute(&mut **transaction)
        .await?;
    sqlx::query("CREATE INDEX user_api_keys_token_auth_idx ON user_api_keys (installation_id, token, revoked_at, expires_at)")
        .execute(&mut **transaction)
        .await?;
    sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)")
        .bind(25_i64)
        .bind(applied_at)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn upgrade_postgres_v24_to_v25(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    applied_at: i64,
) -> Result<(), StorageError> {
    sqlx::query("ALTER TABLE user_api_keys ADD COLUMN token TEXT")
        .execute(&mut **transaction)
        .await?;
    sqlx::query("CREATE UNIQUE INDEX user_api_keys_token_unique_idx ON user_api_keys (installation_id, token)")
        .execute(&mut **transaction)
        .await?;
    sqlx::query("CREATE INDEX user_api_keys_token_auth_idx ON user_api_keys (installation_id, token, revoked_at, expires_at)")
        .execute(&mut **transaction)
        .await?;
    sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES ($1, $2)")
        .bind(25_i64)
        .bind(applied_at)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn upgrade_sqlite_home_pvc(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    applied_at: i64,
) -> Result<(), StorageError> {
    for statement in schema::HOME_PVC_MIGRATION {
        sqlx::query(statement).execute(&mut **transaction).await?;
    }
    sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)")
        .bind(26_i64)
        .bind(applied_at)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn upgrade_postgres_home_pvc(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    applied_at: i64,
) -> Result<(), StorageError> {
    for statement in schema::HOME_PVC_MIGRATION {
        sqlx::query(statement).execute(&mut **transaction).await?;
    }
    sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES ($1, $2)")
        .bind(26_i64)
        .bind(applied_at)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn upgrade_sqlite_runtime_categories(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    applied_at: i64,
) -> Result<(), StorageError> {
    let create = schema::RUNTIME_INCIDENTS_TABLE.replacen(
        "CREATE TABLE workspace_runtime_incidents",
        "CREATE TABLE workspace_runtime_incidents_new",
        1,
    );
    sqlx::query(&create).execute(&mut **transaction).await?;
    for statement in [
        "INSERT INTO workspace_runtime_incidents_new SELECT * FROM workspace_runtime_incidents",
        "DROP TABLE workspace_runtime_incidents",
        "ALTER TABLE workspace_runtime_incidents_new RENAME TO workspace_runtime_incidents",
        "CREATE INDEX workspace_runtime_incidents_recent_idx ON workspace_runtime_incidents (installation_id, workspace_id, last_seen_at, observed_at)",
    ] {
        sqlx::query(statement).execute(&mut **transaction).await?;
    }
    sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)")
        .bind(27_i64)
        .bind(applied_at)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn upgrade_postgres_runtime_categories(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    applied_at: i64,
) -> Result<(), StorageError> {
    sqlx::query("ALTER TABLE workspace_runtime_incidents DROP CONSTRAINT workspace_runtime_incidents_category_check")
        .execute(&mut **transaction)
        .await?;
    sqlx::query("ALTER TABLE workspace_runtime_incidents ADD CONSTRAINT workspace_runtime_incidents_category_check CHECK (category IN ('ephemeral_storage', 'memory_pressure', 'pid_pressure', 'disk_pressure', 'evicted', 'temporary_storage_provisioning', 'temporary_storage_attachment', 'volume_unavailable', 'container_backoff', 'image_pull', 'scheduling', 'health_check', 'other'))")
        .execute(&mut **transaction)
        .await?;
    sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES ($1, $2)")
        .bind(27_i64)
        .bind(applied_at)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn current_sqlite_version(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<i64, StorageError> {
    Ok(
        sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM schema_migrations")
            .fetch_one(&mut **transaction)
            .await?,
    )
}

fn unix_timestamp() -> Result<i64, StorageError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).map_err(|_| StorageError::Clock))
        .map_err(|_| StorageError::Clock)?
}

#[cfg(test)]
#[path = "migration_tests.rs"]
mod current_tests;
