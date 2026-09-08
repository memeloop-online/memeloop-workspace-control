use crate::config::InstallationId;
use sqlx::{PgPool, SqlitePool};

use super::{Database, StorageError, schema};

// Delete this module after every deployed database has crossed schema 20.
mod schema20;

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
        20 => {
            schema20::upgrade_sqlite(&mut transaction, installation_id).await?;
            sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)")
                .bind(schema::SCHEMA_VERSION)
                .bind(applied_at)
                .execute(&mut *transaction)
                .await?;
        }
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
        20 => {
            schema20::upgrade_postgres(&mut transaction, installation_id).await?;
            sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES ($1, $2)")
                .bind(schema::SCHEMA_VERSION)
                .bind(applied_at)
                .execute(&mut *transaction)
                .await?;
        }
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
    use uuid::Uuid;

    const SCHEMA_20_WORKSPACES: &str = "CREATE TABLE workspaces (
        id TEXT PRIMARY KEY,
        installation_id TEXT NOT NULL,
        short_id TEXT NOT NULL,
        runtime_namespace_scope TEXT NOT NULL,
        runtime_namespace TEXT NOT NULL,
        payload TEXT NOT NULL
    )";

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
    async fn unsupported_sqlite_database_is_rejected_without_conversion() {
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
        sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES (19, 1)")
            .execute(pool)
            .await
            .unwrap();
        assert!(matches!(
            database.migrate().await,
            Err(StorageError::UnsupportedDatabaseVersion)
        ));
        assert_eq!(database.schema_version().await.unwrap(), 19);
    }

    #[tokio::test]
    async fn sqlite_schema_20_upgrade_preserves_workspace_rows() {
        let installation: InstallationId = "schema-test".parse().unwrap();
        let database = Database::connect("sqlite::memory:", installation.clone())
            .await
            .unwrap();
        let Database::Sqlite { pool, .. } = &database else {
            unreachable!();
        };
        prepare_sqlite_schema_20(pool).await;
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO workspaces (
                id, installation_id, short_id, runtime_namespace_scope,
                runtime_namespace, payload
            ) VALUES (?1, ?2, ?3, 'dedicated', 'workspace-before-upgrade', 'keep-me')",
        )
        .bind(id.to_string())
        .bind(installation.as_str())
        .bind(crate::workspace_runtime::workspace_short_id_for(id))
        .execute(pool)
        .await
        .unwrap();

        database.migrate().await.unwrap();

        assert_eq!(database.schema_version().await.unwrap(), 21);
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT payload FROM workspaces WHERE id = ?1")
                .bind(id.to_string())
                .fetch_one(pool)
                .await
                .unwrap(),
            "keep-me"
        );
        assert_eq!(
            sqlite_column_count(pool, "runtime_namespace_scope").await,
            0
        );
        assert_eq!(sqlite_column_count(pool, "runtime_namespace").await, 0);
    }

    #[tokio::test]
    async fn sqlite_schema_20_upgrade_rolls_back_every_change_on_ddl_failure() {
        let installation: InstallationId = "schema-test".parse().unwrap();
        let database = Database::connect("sqlite::memory:", installation.clone())
            .await
            .unwrap();
        let Database::Sqlite { pool, .. } = &database else {
            unreachable!();
        };
        prepare_sqlite_schema_20(pool).await;
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO workspaces (
                id, installation_id, short_id, runtime_namespace_scope,
                runtime_namespace, payload
            ) VALUES (?1, ?2, ?3, 'shared', 'workspace-before-upgrade', 'keep-me')",
        )
        .bind(id.to_string())
        .bind(installation.as_str())
        .bind(crate::workspace_runtime::workspace_short_id_for(id))
        .execute(pool)
        .await
        .unwrap();
        sqlx::query("CREATE INDEX prevent_namespace_drop ON workspaces (runtime_namespace)")
            .execute(pool)
            .await
            .unwrap();

        assert!(matches!(
            database.migrate().await,
            Err(StorageError::Database(_))
        ));
        assert_eq!(database.schema_version().await.unwrap(), 20);
        assert_eq!(
            sqlite_column_count(pool, "runtime_namespace_scope").await,
            1
        );
        assert_eq!(sqlite_column_count(pool, "runtime_namespace").await, 1);
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT payload FROM workspaces WHERE id = ?1")
                .bind(id.to_string())
                .fetch_one(pool)
                .await
                .unwrap(),
            "keep-me"
        );
    }

    #[tokio::test]
    async fn postgres_schema_20_upgrade_is_fail_closed_when_configured() {
        let Ok(database_url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
            eprintln!("skipping PostgreSQL migration test: MWC_TEST_POSTGRES_URL is not set");
            return;
        };
        let administration = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .unwrap();

        let valid_schema = format!("mwc_migration_valid_{}", Uuid::now_v7().simple());
        sqlx::query(&format!("CREATE SCHEMA {valid_schema}"))
            .execute(&administration)
            .await
            .unwrap();
        let valid = schema_20_postgres_database(&database_url, &valid_schema, "schema-pg").await;
        let valid_id = insert_postgres_schema_20_workspace(&valid, "schema-pg", "dedicated").await;
        valid.migrate().await.unwrap();
        let Database::Postgres { pool, .. } = &valid else {
            unreachable!();
        };
        assert_eq!(valid.schema_version().await.unwrap(), 21);
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT payload FROM workspaces WHERE id = $1")
                .bind(valid_id.to_string())
                .fetch_one(pool)
                .await
                .unwrap(),
            "keep-me"
        );
        assert_eq!(
            postgres_column_count(pool, "runtime_namespace_scope").await,
            0
        );
        assert_eq!(postgres_column_count(pool, "runtime_namespace").await, 0);
        drop(valid);

        let invalid_schema = format!("mwc_migration_invalid_{}", Uuid::now_v7().simple());
        sqlx::query(&format!("CREATE SCHEMA {invalid_schema}"))
            .execute(&administration)
            .await
            .unwrap();
        let invalid =
            schema_20_postgres_database(&database_url, &invalid_schema, "schema-pg").await;
        let invalid_id =
            insert_postgres_schema_20_workspace(&invalid, "schema-pg", "invalid").await;
        assert!(matches!(
            invalid.migrate().await,
            Err(StorageError::InvalidWorkspace)
        ));
        let Database::Postgres { pool, .. } = &invalid else {
            unreachable!();
        };
        assert_eq!(invalid.schema_version().await.unwrap(), 20);
        assert_eq!(
            postgres_column_count(pool, "runtime_namespace_scope").await,
            1
        );
        assert_eq!(postgres_column_count(pool, "runtime_namespace").await, 1);
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT payload FROM workspaces WHERE id = $1")
                .bind(invalid_id.to_string())
                .fetch_one(pool)
                .await
                .unwrap(),
            "keep-me"
        );
        drop(invalid);

        sqlx::query(&format!(
            "DROP SCHEMA {valid_schema} CASCADE; DROP SCHEMA {invalid_schema} CASCADE"
        ))
        .execute(&administration)
        .await
        .unwrap();
        administration.close().await;
    }

    async fn prepare_sqlite_schema_20(pool: &SqlitePool) {
        sqlx::query(schema::MIGRATION_TABLE)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE installation_metadata (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                installation_id TEXT NOT NULL
            )",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(SCHEMA_20_WORKSPACES)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES (20, 1)")
            .execute(pool)
            .await
            .unwrap();
    }

    async fn sqlite_column_count(pool: &SqlitePool, column: &str) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM pragma_table_info('workspaces') WHERE name = ?1")
            .bind(column)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn schema_20_postgres_database(
        database_url: &str,
        schema_name: &str,
        installation_id: &str,
    ) -> Database {
        let mut scoped_url = url::Url::parse(database_url).unwrap();
        scoped_url
            .query_pairs_mut()
            .append_pair("options", &format!("-c search_path={schema_name}"));
        let database = Database::connect(scoped_url.as_str(), installation_id.parse().unwrap())
            .await
            .unwrap();
        let Database::Postgres { pool, .. } = &database else {
            unreachable!();
        };
        sqlx::query(schema::MIGRATION_TABLE)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE installation_metadata (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                installation_id TEXT NOT NULL
            )",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(SCHEMA_20_WORKSPACES)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES (20, 1)")
            .execute(pool)
            .await
            .unwrap();
        database
    }

    async fn insert_postgres_schema_20_workspace(
        database: &Database,
        installation_id: &str,
        scope: &str,
    ) -> Uuid {
        let Database::Postgres { pool, .. } = database else {
            unreachable!();
        };
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO workspaces (
                id, installation_id, short_id, runtime_namespace_scope,
                runtime_namespace, payload
            ) VALUES ($1, $2, $3, $4, 'workspace-before-upgrade', 'keep-me')",
        )
        .bind(id.to_string())
        .bind(installation_id)
        .bind(crate::workspace_runtime::workspace_short_id_for(id))
        .bind(scope)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    async fn postgres_column_count(pool: &PgPool, column: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM information_schema.columns
             WHERE table_schema = current_schema()
             AND table_name = 'workspaces' AND column_name = $1",
        )
        .bind(column)
        .fetch_one(pool)
        .await
        .unwrap()
    }
}
