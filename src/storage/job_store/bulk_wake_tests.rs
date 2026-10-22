use std::io::Write;

use super::*;

#[tokio::test]
async fn sqlite_bulk_wake_materializes_candidates_and_uses_an_indexed_join() {
    let directory = tempfile::tempdir().unwrap();
    let database = Database::connect(
        &format!(
            "sqlite://{}",
            directory.path().join("wake-plan.sqlite").display()
        ),
        "wake-plan-ci".parse().unwrap(),
    )
    .await
    .unwrap();
    database.migrate().await.unwrap();
    let Database::Sqlite {
        pool,
        installation_id,
    } = &database
    else {
        unreachable!()
    };
    let candidates = serde_json::to_string(
        &(0..3_000)
            .map(|_| {
                serde_json::json!({
                    "id": Uuid::now_v7(),
                    "payload_json": "{\"reconciled_generation\":1}",
                    "until": 160,
                })
            })
            .collect::<Vec<_>>(),
    )
    .unwrap();
    sqlx::query(
        "INSERT INTO jobs \
        (id, installation_id, kind, workspace_id, payload_json, status, available_at, \
        lease_owner, lease_expires_at, attempts, created_at, updated_at) \
        SELECT json_extract(value, '$.id'), ?1, 'reconcile_workspace', NULL, \
        json_extract(value, '$.payload_json'), 'pending', 160, NULL, NULL, 0, 100, 101 \
        FROM json_each(?2)",
    )
    .bind(installation_id.as_str())
    .bind(&candidates)
    .execute(pool)
    .await
    .unwrap();
    let rows = sqlx::query(&format!("EXPLAIN QUERY PLAN {SQLITE_WAKE_PARKED_JOBS}"))
        .bind(103_i64)
        .bind(installation_id.as_str())
        .bind(&candidates)
        .fetch_all(pool)
        .await
        .unwrap();
    let details = rows
        .iter()
        .map(|row| row.get::<String, _>("detail"))
        .collect::<Vec<_>>();
    writeln!(
        std::io::stdout().lock(),
        "MWC_BULK_WAKE_QUERY_PLAN {}",
        serde_json::json!({"backend": "sqlite", "batch": 3000, "details": details})
    )
    .unwrap();
    assert!(
        details
            .iter()
            .any(|detail| detail.contains("MATERIALIZE candidates"))
    );
    assert!(
        details.iter().any(|detail| {
            let detail = detail.to_ascii_lowercase();
            detail.starts_with("search jobs") && detail.contains("index") && detail.contains("id=?")
        }),
        "expected an indexed jobs primary-key lookup: {details:?}"
    );
    pool.close().await;
}
