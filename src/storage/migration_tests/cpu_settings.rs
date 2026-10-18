use super::*;

fn previous_schema(version: i64, installation: &str) -> Vec<String> {
    let mut statements = vec![
        schema::MIGRATION_TABLE.to_owned(),
        format!("INSERT INTO schema_migrations VALUES ({version}, 1)"),
        "CREATE TABLE workspaces (id TEXT PRIMARY KEY, state TEXT NOT NULL)".to_owned(),
        "INSERT INTO workspaces VALUES ('preserved-workspace', 'stopped')".to_owned(),
        "CREATE TABLE user_api_keys (installation_id TEXT, revoked_at BIGINT, expires_at BIGINT)"
            .to_owned(),
    ];
    let incidents = if version < 27 {
        schema::RUNTIME_INCIDENTS_TABLE.replace(
            ", 'container_backoff', 'image_pull', 'scheduling', 'health_check'",
            "",
        )
    } else {
        schema::RUNTIME_INCIDENTS_TABLE.to_owned()
    };
    statements.extend([
        incidents,
        "CREATE INDEX workspace_runtime_incidents_recent_idx ON workspace_runtime_incidents (installation_id, workspace_id, last_seen_at, observed_at)".to_owned(),
        "INSERT INTO workspace_runtime_incidents VALUES ('preserved-event', 'schema-test', 'preserved-workspace', 'other', 10, 6, 10, 20)".to_owned(),
    ]);
    if version >= 26 {
        statements.extend(schema::HOME_PVC_MIGRATION.iter().map(|s| (*s).to_owned()));
    }
    if version >= 28 {
        statements.extend(
            schema::STORAGE_SHRINK_MIGRATION
                .iter()
                .map(|s| (*s).to_owned()),
        );
    }
    if version >= 29 {
        statements.extend(
            schema::RUNTIME_SETTINGS_MIGRATION
                .iter()
                .map(|s| (*s).to_owned()),
        );
        statements.push(format!(
            "INSERT INTO installation_runtime_settings VALUES ('{installation}', 81, 7)"
        ));
    }
    statements
}

#[tokio::test]
async fn sqlite_versions_24_through_29_add_cpu_setting_once_and_preserve_rows() {
    for version in 24..=29 {
        let database = Database::connect("sqlite::memory:", "schema-test".parse().unwrap())
            .await
            .unwrap();
        let Database::Sqlite { pool, .. } = &database else {
            unreachable!()
        };
        for statement in previous_schema(version, "schema-test") {
            sqlx::query(&statement).execute(pool).await.unwrap();
        }
        migrate_sqlite(pool, 100).await.unwrap();
        assert_eq!(database.cpu_overcommit_ratio().await.unwrap(), 10);
        database
            .patch_runtime_settings(None, Some(25), 101)
            .await
            .unwrap();
        migrate_sqlite(pool, 102).await.unwrap();
        assert_eq!(database.schema_version().await.unwrap(), 30);
        assert_eq!(database.cpu_overcommit_ratio().await.unwrap(), 25);
        assert_eq!(
            database.job_worker_concurrency().await.unwrap(),
            if version == 29 { 81 } else { 32 }
        );
        let preserved: i64 = sqlx::query_scalar(
            "SELECT count FROM workspace_runtime_incidents WHERE id = 'preserved-event'",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(preserved, 6);
    }
}

#[tokio::test]
async fn postgres_versions_24_through_29_add_cpu_setting_once_and_preserve_rows() {
    let Ok(url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        return;
    };
    for version in 24..=29 {
        let name = format!("mwc_cpu_upgrade_{}", uuid::Uuid::now_v7().simple());
        let database = postgres_database(&url, &name, None).await;
        let Database::Postgres {
            pool,
            installation_id,
        } = &database
        else {
            unreachable!()
        };
        for statement in previous_schema(version, installation_id.as_str()) {
            sqlx::query(&statement).execute(pool).await.unwrap();
        }
        migrate_postgres(pool, installation_id, 100).await.unwrap();
        assert_eq!(database.cpu_overcommit_ratio().await.unwrap(), 10);
        database
            .patch_runtime_settings(None, Some(25), 101)
            .await
            .unwrap();
        migrate_postgres(pool, installation_id, 102).await.unwrap();
        assert_eq!(database.schema_version().await.unwrap(), 30);
        assert_eq!(database.cpu_overcommit_ratio().await.unwrap(), 25);
        assert_eq!(
            database.job_worker_concurrency().await.unwrap(),
            if version == 29 { 81 } else { 32 }
        );
        let preserved: i64 = sqlx::query_scalar(
            "SELECT count FROM workspace_runtime_incidents WHERE id = 'preserved-event'",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(preserved, 6);
        sqlx::query(&format!("DROP SCHEMA {name} CASCADE"))
            .execute(pool)
            .await
            .unwrap();
        pool.close().await;
    }
}
