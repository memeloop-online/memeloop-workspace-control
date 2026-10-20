use super::*;

#[tokio::test]
async fn current_sqlite_schema_preserves_settings_data_and_history_on_restart() {
    let database = Database::connect("sqlite::memory:", "schema-test".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    assert_eq!(database.cpu_overcommit_ratio().await.unwrap(), 10);
    assert_eq!(database.job_worker_concurrency().await.unwrap(), 32);
    database
        .patch_runtime_settings(Some(81), Some(25), 101)
        .await
        .unwrap();
    let Database::Sqlite { pool, .. } = &database else {
        unreachable!()
    };
    sqlx::query(
        "INSERT INTO organizations VALUES ('preserved-org', 'schema-test', 'Preserved', 1)",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO schema_migrations VALUES (29, 7)")
        .execute(pool)
        .await
        .unwrap();
    let before = database.export_snapshot(102).await.unwrap();
    database.migrate().await.unwrap();
    database.migrate().await.unwrap();
    let after = database.export_snapshot(102).await.unwrap();
    assert_eq!(before.tables, after.tables);
    assert_eq!(after.schema_version, schema::SCHEMA_VERSION);
    assert_eq!(after.installation_id, "schema-test");
    let history: i64 =
        sqlx::query_scalar("SELECT applied_at FROM schema_migrations WHERE version = 29")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(history, 7);
    sqlx::query("SELECT home_pvc_namespace, home_pvc_name, home_pvc_uid, home_pvc_capacity_gib, scratch_volume_binding_json FROM workspaces LIMIT 0")
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn current_postgres_schema_preserves_settings_and_identity_on_restart() {
    let Ok(url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        return;
    };
    let name = format!("mwc_current_settings_{}", uuid::Uuid::now_v7().simple());
    let database = postgres_database(&url, &name, None).await;
    database.migrate().await.unwrap();
    assert_eq!(database.cpu_overcommit_ratio().await.unwrap(), 10);
    assert_eq!(database.job_worker_concurrency().await.unwrap(), 32);
    let settings = database
        .patch_runtime_settings(Some(81), Some(25), 101)
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database.migrate().await.unwrap();
    assert_eq!(database.runtime_settings().await.unwrap(), settings);
    assert_eq!(
        database.schema_version().await.unwrap(),
        schema::SCHEMA_VERSION
    );
    let Database::Postgres { pool, .. } = &database else {
        unreachable!()
    };
    let identity: String =
        sqlx::query_scalar("SELECT installation_id FROM installation_metadata WHERE singleton = 1")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(identity, "schema-pg");
    sqlx::query("SELECT home_pvc_namespace, home_pvc_name, home_pvc_uid, home_pvc_capacity_gib, scratch_volume_binding_json FROM workspaces LIMIT 0")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(&format!("DROP SCHEMA {name} CASCADE"))
        .execute(pool)
        .await
        .unwrap();
    pool.close().await;
}
