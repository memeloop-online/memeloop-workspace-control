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
mod current_tests {
    use super::*;
    use crate::config::InstallationId;

    #[tokio::test]
    async fn fresh_sqlite_bootstraps_schema_22_and_reaccepts_it() {
        let database = Database::connect("sqlite::memory:", "schema-test".parse().unwrap())
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
    async fn pre_22_and_future_sqlite_versions_are_rejected_without_conversion() {
        for version in [20, 21, 23] {
            let installation: InstallationId = "schema-test".parse().unwrap();
            let database = Database::connect("sqlite::memory:", installation)
                .await
                .unwrap();
            let Database::Sqlite { pool, .. } = &database else {
                unreachable!()
            };
            sqlx::query(schema::MIGRATION_TABLE)
                .execute(pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES (?1, 1)")
                .bind(version)
                .execute(pool)
                .await
                .unwrap();
            assert!(matches!(
                database.migrate().await,
                Err(StorageError::UnsupportedDatabaseVersion)
            ));
            assert_eq!(database.schema_version().await.unwrap(), version);
        }
    }

    #[tokio::test]
    async fn postgres_accepts_only_fresh_or_current_schema_when_configured() {
        let Ok(url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
            eprintln!("skipping PostgreSQL migration test: MWC_TEST_POSTGRES_URL is not set");
            return;
        };
        let admin = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        let fresh_schema = format!("mwc_schema_fresh_{}", uuid::Uuid::now_v7().simple());
        let fresh = postgres_database(&url, &fresh_schema, None).await;
        fresh.migrate().await.unwrap();
        assert_eq!(
            fresh.schema_version().await.unwrap(),
            schema::SCHEMA_VERSION
        );
        fresh.migrate().await.unwrap();
        drop(fresh);
        for version in [20, 21, 23] {
            let name = format!("mwc_schema_old_{}", uuid::Uuid::now_v7().simple());
            let database = postgres_database(&url, &name, Some(version)).await;
            assert!(matches!(
                database.migrate().await,
                Err(StorageError::UnsupportedDatabaseVersion)
            ));
            assert_eq!(database.schema_version().await.unwrap(), version);
            drop(database);
            sqlx::query(&format!("DROP SCHEMA {name} CASCADE"))
                .execute(&admin)
                .await
                .unwrap();
        }
        sqlx::query(&format!("DROP SCHEMA {fresh_schema} CASCADE"))
            .execute(&admin)
            .await
            .unwrap();
        admin.close().await;
    }

    async fn postgres_database(url: &str, schema_name: &str, version: Option<i64>) -> Database {
        let admin = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(url)
            .await
            .unwrap();
        sqlx::query(&format!("CREATE SCHEMA {schema_name}"))
            .execute(&admin)
            .await
            .unwrap();
        admin.close().await;
        let mut scoped = url::Url::parse(url).unwrap();
        scoped
            .query_pairs_mut()
            .append_pair("options", &format!("-c search_path={schema_name}"));
        let database = Database::connect(scoped.as_str(), "schema-pg".parse().unwrap())
            .await
            .unwrap();
        if let Some(version) = version {
            let Database::Postgres { pool, .. } = &database else {
                unreachable!()
            };
            sqlx::query(schema::MIGRATION_TABLE)
                .execute(pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES ($1, 1)")
                .bind(version)
                .execute(pool)
                .await
                .unwrap();
        }
        database
    }
}
