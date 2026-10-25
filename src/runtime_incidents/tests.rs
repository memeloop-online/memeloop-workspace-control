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
        "metadata": {"uid":"90000000-0000-4000-8000-000000000001", "creationTimestamp": "2026-09-16T10:00:00Z"},
        "involvedObject": {"uid":"80000000-0000-4000-8000-000000000001", "kind": "Pod", "namespace": WORKSPACE_NAMESPACE, "name": "w-8000000000000001-0"},
        "reason": "Evicted", "message": message, "count": 2, "type": "Warning"
    })).unwrap()
}

#[test]
fn only_this_pods_missing_startup_marker_is_expected_waiting() {
    let expected = "MountVolume.SetUp failed for volume \"workspace-startup-gate\" : configmap \"w-8000000000000001-startup-gate\" not found";
    let mut waiting = event(expected);
    waiting.reason = Some("FailedMount".to_owned());
    assert!(startup_gate_pending(&waiting));
    assert!(normalized_incident(&waiting).is_none());
    for unexpected in [
        expected.replace("8000000000000001", "8000000000000002"),
        expected.replace("not found", "permission denied"),
        expected.replace("workspace-startup-gate", "workspace-config"),
        expected.replace("configmap", "secret"),
    ] {
        waiting.message = Some(unexpected);
        assert!(!startup_gate_pending(&waiting));
        assert!(normalized_incident(&waiting).is_some());
    }
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
    event.type_ = Some("Normal".into());
    assert!(normalized_incident(&event).is_none());
    event.type_ = Some("Warning".into());
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
    sqlx::query("CREATE TABLE workspace_runtime_incidents (id TEXT PRIMARY KEY, installation_id TEXT NOT NULL, workspace_id TEXT NOT NULL REFERENCES workspaces(id), category TEXT NOT NULL, observed_at BIGINT NOT NULL, count BIGINT NOT NULL, first_seen_at BIGINT NOT NULL, last_seen_at BIGINT NOT NULL CHECK(last_seen_at >= first_seen_at), details_json TEXT NOT NULL)").execute(pool).await.unwrap();
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
    let mut sync_failure = event("private kubelet sync diagnostic");
    sync_failure.metadata.uid = Some("90000000-0000-4000-8000-000000000002".into());
    sync_failure.reason = Some("FailedSync".into());
    let sync_incident = normalized_incident(&sync_failure).unwrap();
    assert_eq!(sync_incident.category, RuntimeEventCategory::Other);
    database
        .upsert_workspace_runtime_incidents(workspace_id, std::slice::from_ref(&sync_incident), now)
        .await
        .unwrap();
    let history = database
        .list_workspace_runtime_incidents(&[workspace_id], now + 7200, 50)
        .await
        .unwrap();
    let stored = history[&workspace_id]
        .iter()
        .find(|row| row.id == sync_incident.id)
        .unwrap();
    assert_eq!(stored.details.reason.as_deref(), Some("FailedSync"));
    assert!(
        !serde_json::to_string(&stored.details)
            .unwrap()
            .contains("private")
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

#[test]
fn startup_failures_are_actionable_and_unknown_warnings_are_not_stored() {
    for (reason, message, expected) in [
        (
            "BackOff",
            "Back-off restarting failed container ttyd",
            RuntimeEventCategory::ContainerBackoff,
        ),
        (
            "Failed",
            "Failed to pull image",
            RuntimeEventCategory::ImagePull,
        ),
        (
            "FailedScheduling",
            "0 nodes available: waiting for ephemeral volume controller to create persistentvolumeclaim workspace-scratch",
            RuntimeEventCategory::TemporaryStorageProvisioning,
        ),
        (
            "FailedScheduling",
            "Insufficient cpu",
            RuntimeEventCategory::Scheduling,
        ),
        (
            "Unhealthy",
            "Readiness probe failed",
            RuntimeEventCategory::HealthCheck,
        ),
    ] {
        let mut source = event(message);
        source.reason = Some(reason.to_owned());
        let incident = normalized_incident(&source).unwrap();
        assert_eq!(incident.category, expected);
    }
    let mut unknown = event("private arbitrary message");
    unknown.reason = Some("UnknownWarning".into());
    assert!(normalized_incident(&unknown).is_none());
}

#[test]
fn normalization_keeps_exact_source_and_pod_identity_with_bounded_details() {
    let mut source =
        event("Container workspace exceeded its local ephemeral storage limit \"2Gi\". ");
    source.involved_object.field_path = Some("spec.containers{workspace}".into());
    let incident = normalized_incident(&source).unwrap();
    assert_eq!(incident.id, "90000000-0000-4000-8000-000000000001");
    assert_eq!(incident.details.event_uid.unwrap().to_string(), incident.id);
    assert_eq!(
        incident.details.pod_uid.unwrap().to_string(),
        "80000000-0000-4000-8000-000000000001"
    );
    assert_eq!(incident.details.reason.as_deref(), Some("Evicted"));
    assert_eq!(
        incident.details.container,
        Some(RuntimeContainer::Workspace)
    );
    assert_eq!(
        incident.details.diagnostic,
        Some(RuntimeDiagnostic::ContainerLocalEphemeralLimit {
            container: RuntimeContainer::Workspace,
            limit_bytes: 2 * 1024 * 1024 * 1024,
        })
    );
    source.involved_object.field_path = Some("spec.initContainers{workspace-bootstrap}".into());
    assert_eq!(
        event_details(&source).container,
        Some(RuntimeContainer::WorkspaceBootstrap)
    );
    source.involved_object.field_path = Some("spec.containers{private-token}".into());
    source.involved_object.uid = Some("private-pod-token".into());
    source.reason = Some("private-reason-token".into());
    source.message = Some("Readiness probe failed: https://private-token.invalid/secret".into());
    let details = event_details(&source);
    assert!(details.container.is_none());
    assert!(details.pod_uid.is_none());
    assert!(details.reason.is_none());
    assert!(details.diagnostic.is_none());
    assert!(!serde_json::to_string(&details).unwrap().contains("private"));
    source.metadata.uid = None;
    assert!(normalized_incident(&source).is_none());
}
