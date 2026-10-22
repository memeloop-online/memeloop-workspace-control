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
    for name in ["candidates", "matched"] {
        assert!(
            details
                .iter()
                .any(|detail| detail == &format!("MATERIALIZE {name}"))
        );
    }
    let is_parked_id_lookup = |detail: &str| {
        let detail = detail.to_ascii_lowercase();
        let mut words = detail.split_whitespace();
        words.next() == Some("search")
            && words.next() == Some("parked")
            && detail.contains("index")
            && detail
                .split(['(', ')', ' ', ','])
                .any(|token| token == "id=?")
    };
    assert!(!is_parked_id_lookup(
        "SEARCH parked USING INDEX jobs_claim_idx (installation_id=? AND status=? AND available_at>?)"
    ));
    assert!(!is_parked_id_lookup(
        "SEARCH jobs USING INDEX sqlite_autoindex_jobs_1 (id=?)"
    ));
    assert!(is_parked_id_lookup(
        "SEARCH parked USING INDEX sqlite_autoindex_jobs_1 (id=?)"
    ));
    let candidate_scan = details
        .iter()
        .position(|detail| detail.split_whitespace().take(2).eq(["SCAN", "candidate"]))
        .expect("matched must scan candidates as its outer loop");
    let parked_lookup = details
        .iter()
        .position(|detail| is_parked_id_lookup(detail))
        .unwrap_or_else(|| panic!("expected an indexed parked.id lookup: {details:?}"));
    assert!(
        candidate_scan < parked_lookup,
        "candidate must drive the parked lookup: {details:?}"
    );
    assert_eq!(
        rows[candidate_scan].get::<i64, _>("parent"),
        rows[parked_lookup].get::<i64, _>("parent"),
        "candidate scan and parked lookup must belong to the same matched join"
    );
    pool.close().await;
}
