use super::*;

async fn exercise_settings(database: &Database) {
    database.migrate().await.unwrap();
    assert_eq!(
        database.runtime_settings().await.unwrap(),
        RuntimeSettings::default()
    );
    // Both partial writes can race to create the singleton row.
    let (concurrency, cpu) = tokio::join!(
        database.patch_runtime_settings(Some(64), None, 1),
        database.patch_runtime_settings(None, Some(20), 1)
    );
    concurrency.unwrap();
    cpu.unwrap();
    assert_eq!(database.job_worker_concurrency().await.unwrap(), 64);
    assert_eq!(database.cpu_overcommit_ratio().await.unwrap(), 20);
    let settings = database
        .patch_runtime_settings(Some(64), Some(20), 1)
        .await
        .unwrap();
    assert_eq!(
        settings,
        RuntimeSettings {
            job_worker_concurrency: 64,
            cpu_overcommit_ratio: 20
        }
    );
    // Invalid combined updates are rejected before either field is written.
    for ratio in [0, 1001] {
        assert!(matches!(
            database
                .patch_runtime_settings(Some(70), Some(ratio), 2)
                .await,
            Err(StorageError::InvalidCpuOvercommitRatio)
        ));
    }
    assert_eq!(database.runtime_settings().await.unwrap(), settings);
    let (concurrency, cpu) = tokio::join!(
        database.patch_runtime_settings(Some(96), None, 3),
        database.patch_runtime_settings(None, Some(1000), 3)
    );
    concurrency.unwrap();
    cpu.unwrap();
    assert_eq!(
        database.runtime_settings().await.unwrap(),
        RuntimeSettings {
            job_worker_concurrency: 96,
            cpu_overcommit_ratio: 1000
        }
    );
    database.set_job_worker_concurrency(1, 4).await.unwrap();
    assert_eq!(database.cpu_overcommit_ratio().await.unwrap(), 1000);
    database
        .patch_runtime_settings(None, Some(1), 5)
        .await
        .unwrap();
    assert_eq!(database.job_worker_concurrency().await.unwrap(), 1);
    assert_eq!(database.cpu_overcommit_ratio().await.unwrap(), 1);
    assert_eq!(database.job_counts().await.unwrap().pending, 0);
}

#[tokio::test]
async fn sqlite_defaults_patches_and_concurrent_updates_preserve_other_fields() {
    let database = Database::connect("sqlite::memory:", "runtime-test".parse().unwrap())
        .await
        .unwrap();
    exercise_settings(&database).await;
    let Database::Sqlite { pool, .. } = &database else {
        unreachable!()
    };
    let other = Database::Sqlite {
        pool: pool.clone(),
        installation_id: "other-runtime".parse().unwrap(),
    };
    assert_eq!(
        other.runtime_settings().await.unwrap(),
        RuntimeSettings::default()
    );
    other
        .patch_runtime_settings(None, Some(8), 6)
        .await
        .unwrap();
    assert_eq!(other.job_worker_concurrency().await.unwrap(), 32);
    assert_eq!(database.cpu_overcommit_ratio().await.unwrap(), 1);
    for ratio in [0, 1001] {
        assert!(
            sqlx::query("UPDATE installation_runtime_settings SET cpu_overcommit_ratio = ?1")
                .bind(ratio)
                .execute(pool)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn postgres_defaults_patches_and_concurrent_updates_preserve_other_fields() {
    let Ok(url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        return;
    };
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    let schema = format!("mwc_runtime_{}", uuid::Uuid::now_v7().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let mut scoped = url::Url::parse(&url).unwrap();
    scoped
        .query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema}"));
    let database = Database::connect(scoped.as_str(), "runtime-pg".parse().unwrap())
        .await
        .unwrap();
    exercise_settings(&database).await;
    let Database::Postgres { pool, .. } = &database else {
        unreachable!()
    };
    for ratio in [0, 1001] {
        assert!(
            sqlx::query("UPDATE installation_runtime_settings SET cpu_overcommit_ratio = $1")
                .bind(i64::from(ratio))
                .execute(pool)
                .await
                .is_err()
        );
    }
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
