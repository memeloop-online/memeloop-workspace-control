use crate::config::InstallationId;
use sqlx::{PgPool, SqlitePool};

use super::{Database, StorageError, schema};

pub(super) async fn migrate(database: &Database) -> Result<(), StorageError> {
    let applied_at = unix_timestamp()?;
    match database {
        Database::Sqlite {
            pool,
            installation_id,
        } => migrate_sqlite(pool, installation_id, applied_at).await?,
        Database::Postgres {
            pool,
            installation_id,
        } => migrate_postgres(pool, installation_id, applied_at).await?,
    }
    database.ensure_installation_identity().await
}

async fn migrate_sqlite(
    pool: &SqlitePool,
    installation_id: &InstallationId,
    applied_at: i64,
) -> Result<(), StorageError> {
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
        schema::SCHEMA_VERSION => {}
        0 if has_application_tables == 0 => {
            for statement in schema::BASELINE {
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
    transaction.commit().await?;
    let _ = installation_id;
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
        schema::SCHEMA_VERSION => {}
        0 if has_application_tables == 0 => {
            for statement in schema::BASELINE {
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
    transaction.commit().await?;
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
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_sqlite_creates_current_schema_and_accepts_it_again() {
        let installation: InstallationId = "schema-test".parse().unwrap();
        let database = Database::connect("sqlite::memory:", installation)
            .await
            .unwrap();
        database.migrate().await.unwrap();
        assert_eq!(
            database.schema_version().await.unwrap(),
            schema::SCHEMA_VERSION
        );
        database.migrate().await.unwrap();
    }

    #[tokio::test]
    async fn earlier_sqlite_database_is_rejected_without_conversion() {
        let installation: InstallationId = "schema-test".parse().unwrap();
        let database = Database::connect("sqlite::memory:", installation)
            .await
            .unwrap();
        let Database::Sqlite { pool, .. } = &database else {
            unreachable!();
        };
        sqlx::query(schema::MIGRATION_TABLE)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES (18, 1)")
            .execute(pool)
            .await
            .unwrap();
        assert!(matches!(
            database.migrate().await,
            Err(StorageError::UnsupportedDatabaseVersion)
        ));
    }
}
