use super::*;

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
    for version in (0..schema::SCHEMA_VERSION).chain([schema::SCHEMA_VERSION + 1]) {
        let database = Database::connect("sqlite::memory:", "schema-test".parse().unwrap())
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
    for version in (0..schema::SCHEMA_VERSION).chain([schema::SCHEMA_VERSION + 1]) {
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
    let name = format!("mwc_schema_unversioned_{}", uuid::Uuid::now_v7().simple());
    let database = postgres_database(&url, &name, None).await;
    let Database::Postgres { pool, .. } = &database else {
        unreachable!()
    };
    sqlx::query("CREATE TABLE preserved (value TEXT)")
        .execute(pool)
        .await
        .unwrap();
    assert!(matches!(
        database.migrate().await,
        Err(StorageError::UnsupportedDatabaseVersion)
    ));
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables WHERE table_schema = current_schema() ORDER BY table_name",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(tables, vec!["preserved"]);
    sqlx::query(&format!("DROP SCHEMA {name} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
    pool.close().await;
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

#[tokio::test]
async fn unversioned_nonempty_sqlite_database_is_rejected() {
    let database = Database::connect("sqlite::memory:", "schema-test".parse().unwrap())
        .await
        .unwrap();
    let Database::Sqlite { pool, .. } = &database else {
        unreachable!()
    };
    sqlx::query("CREATE TABLE preserved (value TEXT)")
        .execute(pool)
        .await
        .unwrap();
    assert!(matches!(
        database.migrate().await,
        Err(StorageError::UnsupportedDatabaseVersion)
    ));
    let tables: Vec<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .fetch_all(pool)
            .await
            .unwrap();
    assert_eq!(tables, vec!["preserved"]);
}

#[path = "migration_tests/cpu_settings.rs"]
mod cpu_settings;
