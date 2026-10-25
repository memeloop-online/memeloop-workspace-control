use super::*;
use k8s_openapi::{
    api::core::v1::PersistentVolumeClaim, apimachinery::pkg::apis::meta::v1::ObjectMeta,
};

#[test]
fn workspace_id_query_is_deduplicated_and_bounded() {
    let first = Uuid::parse_str("01a05874-0f29-78f2-95ca-086b4debca09").unwrap();
    let second = Uuid::parse_str("01a05875-87b8-74c1-a252-412a71050991").unwrap();
    let parsed = parse_workspace_ids(&format!("{second},{first},{second}")).unwrap();
    assert_eq!(parsed, vec![first, second]);

    assert!(parse_workspace_ids("").is_err());
    assert!(parse_workspace_ids("not-a-uuid").is_err());
    let too_many = (0..101)
        .map(|_| Uuid::now_v7().to_string())
        .collect::<Vec<_>>()
        .join(",");
    assert!(parse_workspace_ids(&too_many).is_err());
}

#[test]
fn pvc_observation_uses_explicit_storage_role() {
    let workspace_id = Uuid::now_v7();
    let labels = |role: &str| {
        BTreeMap::from([
            (WORKSPACE_ID_LABEL.to_owned(), workspace_id.to_string()),
            (STORAGE_ROLE_LABEL.to_owned(), role.to_owned()),
        ])
    };
    let pvc = |name: &str, role: &str| PersistentVolumeClaim {
        metadata: ObjectMeta {
            name: Some(name.to_owned()),
            namespace: Some("workspaces".to_owned()),
            labels: Some(labels(role)),
            ..ObjectMeta::default()
        },
        ..PersistentVolumeClaim::default()
    };

    let observed = index_storage_pvcs(vec![
        pvc("workspace-data-w-example-0", STORAGE_ROLE_HOME),
        pvc("w-example-0-workspace-scratch", STORAGE_ROLE_TEMPORARY),
        pvc("unclassified", "other"),
    ]);
    let storage = observed.get(&workspace_id).unwrap();
    assert_eq!(
        storage.persistent.as_ref(),
        Some(&(
            "workspaces".to_owned(),
            "workspace-data-w-example-0".to_owned()
        ))
    );
    assert_eq!(
        storage.temporary.as_ref(),
        Some(&(
            "workspaces".to_owned(),
            "w-example-0-workspace-scratch".to_owned()
        ))
    );
}

#[test]
fn runtime_history_replays_complete_details_after_kubernetes_event_expiry() {
    let source: k8s_openapi::api::core::v1::Event = serde_json::from_value(serde_json::json!({
        "metadata": {"uid":"90000000-0000-4000-8000-000000000001"},
        "involvedObject": {"uid":"80000000-0000-4000-8000-000000000001", "fieldPath":"spec.containers{workspace}"},
        "reason":"Evicted", "message":"Container workspace exceeded its local ephemeral storage limit \"2Gi\". ",
        "eventTime":"2026-09-15T12:30:00Z", "count":4
    })).unwrap();
    let mut sync_source = source.clone();
    sync_source.reason = Some("FailedSync".into());
    sync_source.message = Some("private kubelet sync diagnostic".into());
    for source in [source, sync_source] {
        let event = pod_event(source).unwrap();
        let incident = new_runtime_incident(&event).unwrap();
        let stored = WorkspaceRuntimeIncident {
            id: incident.id,
            details: incident.details,
            workspace_id: Uuid::now_v7(),
            last_observed_at: incident.last_observed_at,
            category: incident.category,
            observed_at: incident.observed_at,
            count: incident.count,
        };
        // No live Events remain: history itself supplies the known reason,
        // Pod identity and parsed facts, including informative Other warnings.
        assert_eq!(merge_current_events(vec![stored], &[]), vec![event]);
    }
}

#[test]
fn event_identity_keeps_same_second_events_distinct_and_does_not_borrow_details() {
    let at = "2026-10-18T12:14:00Z"
        .parse::<k8s_openapi::jiff::Timestamp>()
        .unwrap()
        .as_second();
    let current = PodEvent {
        id: "pod-condition:80000000-0000-4000-8000-000000000001:PodScheduled:2026-10-18T12:14:00Z"
            .into(),
        category: RuntimeEventCategory::Scheduling,
        count: None,
        observed_at: Some("2026-10-18T12:14:00Z".into()),
        last_observed_at: Some("2026-10-18T12:14:00Z".into()),
        details: crate::runtime_incidents::RuntimeEventDetails {
            reason: Some("FailedScheduling".into()),
            pod_uid: Some("80000000-0000-4000-8000-000000000001".parse().unwrap()),
            scheduling_blockers: vec![crate::runtime_incidents::SchedulingBlocker::CpuCapacity],
            ..Default::default()
        },
    };
    let historical = WorkspaceRuntimeIncident {
        id: Uuid::now_v7().to_string(),
        details: Default::default(),
        workspace_id: Uuid::now_v7(),
        category: RuntimeEventCategory::Scheduling,
        observed_at: at,
        last_observed_at: at,
        count: 7,
    };
    let historical_id = historical.id.clone();
    let merged = merge_current_events(vec![historical], &[current.clone(), current]);
    assert_eq!(merged.len(), 2);
    let historical = merged
        .iter()
        .find(|event| event.id == historical_id)
        .unwrap();
    assert_eq!(historical.count, Some(7));
    assert!(historical.details.event_uid.is_none());
    assert!(historical.details.scheduling_blockers.is_empty());
}

#[test]
fn same_source_uses_latest_facts_without_decreasing_count_or_changing_start() {
    use crate::runtime_incidents::{RuntimeEventDetails, SchedulingBlocker};
    let historical = WorkspaceRuntimeIncident {
        id: Uuid::now_v7().to_string(),
        workspace_id: Uuid::now_v7(),
        category: RuntimeEventCategory::Scheduling,
        observed_at: 2_000_000_000,
        last_observed_at: 2_000_000_010,
        count: 5,
        details: RuntimeEventDetails {
            reason: Some("FailedScheduling".into()),
            scheduling_blockers: vec![SchedulingBlocker::StorageCapacity],
            ..Default::default()
        },
    };
    let mut live = pod_event_from_incident(historical.clone());
    live.last_observed_at = Some(
        k8s_openapi::jiff::Timestamp::new(2_000_000_020, 0)
            .unwrap()
            .to_string(),
    );
    live.count = Some(2);
    live.details.scheduling_blockers = vec![SchedulingBlocker::CpuCapacity];
    let mut stale = pod_event_from_incident(historical.clone());
    stale.count = Some(7);
    let expected_details = live.details.clone();
    let events = merge_current_events(vec![historical.clone()], &[live, stale]);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id, historical.id);
    assert_eq!(events[0].count, Some(7));
    assert_eq!(events[0].details, expected_details);
    assert_eq!(
        new_runtime_incident(&events[0]).unwrap().observed_at,
        historical.observed_at
    );
}
