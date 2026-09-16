use super::*;
use axum::{
    body::Body,
    http::{Request, Response},
};
use serde_json::json;
use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
};

fn event(message: &str) -> Event {
    serde_json::from_value(json!({
        "metadata": {"creationTimestamp": "2026-09-16T10:00:00Z"},
        "involvedObject": {"kind": "Pod", "namespace": WORKSPACE_NAMESPACE, "name": "w-8000000000000001-0"},
        "reason": "Evicted", "message": message, "count": 2, "type": "Warning"
    })).unwrap()
}

#[test]
fn eviction_resource_causes_take_precedence_over_generic_evicted() {
    for (message, expected) in [
        (
            "The node was low on resource: ephemeral-storage. secret-node",
            RuntimeEventCategory::EphemeralStorage,
        ),
        (
            "Container exceeded ephemeral storage limit",
            RuntimeEventCategory::EphemeralStorage,
        ),
        (
            "The node was low on resource: memory.",
            RuntimeEventCategory::MemoryPressure,
        ),
        ("MemoryPressure", RuntimeEventCategory::MemoryPressure),
        ("node had DiskPressure", RuntimeEventCategory::DiskPressure),
        ("node had PIDPressure", RuntimeEventCategory::PidPressure),
        (
            "The node was low on resource: pid.",
            RuntimeEventCategory::PidPressure,
        ),
        ("Unclassified eviction", RuntimeEventCategory::Evicted),
    ] {
        let incident = normalized_incident(&event(message)).unwrap();
        assert_eq!(incident.category, expected);
        assert_eq!(incident.count, 2);
        assert!(!format!("{incident:?}").contains("secret-node"));
    }
}

#[test]
fn mapping_survives_pod_deletion_but_rejects_unrelated_names_and_namespaces() {
    let mut event = event("evicted");
    assert_eq!(workspace_short_id(&event), Some("8000000000000001"));
    for name in ["w-8000000000000001-1", "w-other-0", "w-800000000000000G-0"] {
        event.involved_object.name = Some(name.into());
        assert_eq!(workspace_short_id(&event), None);
    }
    event.involved_object.name = Some("w-8000000000000001-0".into());
    event.involved_object.namespace = Some("other".into());
    assert_eq!(workspace_short_id(&event), None);
}

#[test]
fn normal_events_and_missing_timestamps_are_not_persisted() {
    let mut event = event("Successfully started container");
    event.reason = Some("Started".into());
    assert!(normalized_incident(&event).is_none());
    event.reason = Some("Evicted".into());
    event.metadata.creation_timestamp = None;
    assert!(normalized_incident(&event).is_none());
}

#[test]
fn successful_volume_events_never_become_background_incidents() {
    let mut event = event("AttachVolume.Attach succeeded for volume workspace-scratch");
    event.reason = Some("SuccessfulAttachVolume".into());
    event.type_ = Some("Normal".into());
    assert!(normalized_incident(&event).is_none());
    event.type_ = None;
    assert!(normalized_incident(&event).is_none());
}

#[tokio::test]
async fn consecutive_series_observations_update_one_history_row() {
    let database = Database::connect("sqlite::memory:", "series-test".parse().unwrap())
        .await
        .unwrap();
    let Database::Sqlite { pool, .. } = &database else {
        unreachable!()
    };
    // A minimal parent table isolates this ingestion/upsert contract from
    // unrelated workspace provisioning and admission fixtures.
    sqlx::query("CREATE TABLE workspaces (id TEXT PRIMARY KEY)")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE workspace_runtime_incidents (id TEXT PRIMARY KEY, installation_id TEXT NOT NULL, workspace_id TEXT NOT NULL REFERENCES workspaces(id), category TEXT NOT NULL, observed_at BIGINT NOT NULL, count BIGINT NOT NULL, first_seen_at BIGINT NOT NULL, last_seen_at BIGINT NOT NULL CHECK(last_seen_at >= first_seen_at), UNIQUE(installation_id, workspace_id, category, observed_at))").execute(pool).await.unwrap();
    let workspace_id = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO workspaces VALUES (?1)")
        .bind(workspace_id.to_string())
        .execute(pool)
        .await
        .unwrap();
    let mut first = event("The node was low on resource: ephemeral-storage.");
    first.series = Some(
        serde_json::from_value(json!({"count": 2, "lastObservedTime":"2026-09-16T10:00:10Z"}))
            .unwrap(),
    );
    let mut second = first.clone();
    second.series = Some(
        serde_json::from_value(json!({"count": 5, "lastObservedTime":"2026-09-16T10:00:40Z"}))
            .unwrap(),
    );
    let now = "2026-09-16T10:01:00Z"
        .parse::<k8s_openapi::jiff::Timestamp>()
        .unwrap()
        .as_second();
    for observation in [&first, &second, &first] {
        database
            .upsert_workspace_runtime_incidents(
                workspace_id,
                &[normalized_incident(observation).unwrap()],
                now,
            )
            .await
            .unwrap();
    }
    use sqlx::Row;
    let rows =
        sqlx::query("SELECT observed_at, count, last_seen_at FROM workspace_runtime_incidents")
            .fetch_all(pool)
            .await
            .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get::<i64, _>("count"), 5);
    assert_eq!(
        rows[0].get::<i64, _>("observed_at"),
        event_start(&first).unwrap()
    );
    assert_eq!(
        rows[0].get::<i64, _>("last_seen_at"),
        normalized_incident(&second).unwrap().last_observed_at
    );
}

#[tokio::test]
async fn collector_pages_namespace_events_without_querying_individual_pods() {
    let database = Database::connect("sqlite::memory:", "collector-test".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let service = tower::service_fn(move |request: Request<kube::client::Body>| {
        let captured = captured.clone();
        async move {
            let mut requests = captured.lock().unwrap();
            requests.push(request.uri().to_string());
            let cursor = if requests.len() == 1 { "next-page" } else { "" };
            // Workspace is absent from this installation: do not persist it.
            let body = json!({"apiVersion":"v1", "kind":"EventList", "metadata":{"continue": cursor}, "items":[event("ephemeral-storage")]});
            Ok::<_, Infallible>(
                Response::builder()
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
        }
    });
    let api = Api::namespaced(
        kube::Client::new(service, WORKSPACE_NAMESPACE),
        WORKSPACE_NAMESPACE,
    );
    collect_once(&api, &database).await.unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests.iter().all(
            |uri| uri.starts_with(&format!("/api/v1/namespaces/{WORKSPACE_NAMESPACE}/events?"))
        )
    );
    assert!(requests[1].contains("continue=next-page"));
    assert!(
        requests
            .iter()
            .all(|uri| !uri.contains("involvedObject.name"))
    );
}
