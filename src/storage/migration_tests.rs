use super::*;
use crate::config::InstallationId;

#[tokio::test]
async fn fresh_sqlite_bootstraps_current_schema_and_reaccepts_it() {
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
async fn unsupported_sqlite_versions_are_rejected_without_conversion() {
    for version in [20, 21, 22, 23] {
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
    for version in [20, 21, 22, 23] {
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

fn runtime_v26_fixture() -> Vec<String> {
    vec![
        schema::MIGRATION_TABLE.to_owned(),
        "INSERT INTO schema_migrations (version, applied_at) VALUES (26, 1)".to_owned(),
        "CREATE TABLE workspaces (id TEXT PRIMARY KEY)".to_owned(),
        "INSERT INTO workspaces (id) VALUES ('workspace-1')".to_owned(),
        schema::RUNTIME_INCIDENTS_TABLE.replace(
            ", 'container_backoff', 'image_pull', 'scheduling', 'health_check'", ""),
        "CREATE INDEX workspace_runtime_incidents_recent_idx ON workspace_runtime_incidents (installation_id, workspace_id, last_seen_at, observed_at)".to_owned(),
        "INSERT INTO workspace_runtime_incidents VALUES ('existing', 'schema-test', 'workspace-1', 'other', 10, 6, 10, 20)".to_owned(),
    ]
}

fn new_incident_insert(category: &str) -> String {
    format!(
        "INSERT INTO workspace_runtime_incidents VALUES ('{category}', 'schema-test', 'workspace-1', '{category}', 30, 1, 30, 30)"
    )
}

#[tokio::test]
async fn sqlite_v26_preserves_events_and_accepts_new_categories() {
    let database = Database::connect("sqlite::memory:", "schema-test".parse().unwrap())
        .await
        .unwrap();
    let Database::Sqlite { pool, .. } = &database else {
        unreachable!()
    };
    for statement in runtime_v26_fixture() {
        sqlx::query(&statement).execute(pool).await.unwrap();
    }
    migrate_sqlite(pool, 100).await.unwrap();
    migrate_sqlite(pool, 101).await.unwrap();
    assert_eq!(
        database.schema_version().await.unwrap(),
        schema::SCHEMA_VERSION
    );
    let existing: (i64, i64) = sqlx::query_as(
        "SELECT count, last_seen_at FROM workspace_runtime_incidents WHERE id = 'existing'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(existing, (6, 20));
    for category in [
        "container_backoff",
        "image_pull",
        "scheduling",
        "health_check",
    ] {
        sqlx::query(&new_incident_insert(category))
            .execute(pool)
            .await
            .unwrap();
    }
    assert!(
        sqlx::query(&new_incident_insert("unknown"))
            .execute(pool)
            .await
            .is_err()
    );
    sqlx::query("DELETE FROM workspaces")
        .execute(pool)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workspace_runtime_incidents")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn postgres_v26_preserves_events_and_accepts_new_categories() {
    let Ok(url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        return;
    };
    let name = format!("mwc_schema_events_{}", uuid::Uuid::now_v7().simple());
    let database = postgres_database(&url, &name, None).await;
    let Database::Postgres {
        pool,
        installation_id,
    } = &database
    else {
        unreachable!()
    };
    for statement in runtime_v26_fixture() {
        sqlx::query(&statement).execute(pool).await.unwrap();
    }
    migrate_postgres(pool, installation_id, 100).await.unwrap();
    migrate_postgres(pool, installation_id, 101).await.unwrap();
    assert_eq!(
        database.schema_version().await.unwrap(),
        schema::SCHEMA_VERSION
    );
    let existing: (i64, i64) = sqlx::query_as(
        "SELECT count, last_seen_at FROM workspace_runtime_incidents WHERE id = 'existing'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(existing, (6, 20));
    for category in [
        "container_backoff",
        "image_pull",
        "scheduling",
        "health_check",
    ] {
        sqlx::query(&new_incident_insert(category))
            .execute(pool)
            .await
            .unwrap();
    }
    assert!(
        sqlx::query(&new_incident_insert("unknown"))
            .execute(pool)
            .await
            .is_err()
    );
    sqlx::query("DELETE FROM workspaces")
        .execute(pool)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workspace_runtime_incidents")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    sqlx::query(&format!("DROP SCHEMA {name} CASCADE"))
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn sqlite_v28_adds_runtime_settings_without_reapplying_storage_shrink_columns() {
    let database = Database::connect("sqlite::memory:", "runtime-upgrade".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let Database::Sqlite { pool, .. } = &database else {
        unreachable!()
    };
    sqlx::query("DROP TABLE installation_runtime_settings")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM schema_migrations")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES (28, 1)")
        .execute(pool)
        .await
        .unwrap();
    database.migrate().await.unwrap();
    assert_eq!(database.job_worker_concurrency().await.unwrap(), 32);
    database.set_job_worker_concurrency(100, 2).await.unwrap();
    database.migrate().await.unwrap();
    assert_eq!(database.job_worker_concurrency().await.unwrap(), 100);
    let snapshot = database.export_snapshot(3).await.unwrap();
    assert_eq!(
        snapshot.tables["installation_runtime_settings"][0]["job_worker_concurrency"],
        100
    );
}

#[path = "migration_tests/cpu_settings.rs"]
mod cpu_settings;
