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
fn runtime_incident_round_trip_keeps_only_normalized_fields() {
    let event = PodEvent {
        reason: None,
        component: None,
        scheduling_blockers: Vec::new(),
        last_observed_at: Some("2026-09-15T12:30:00Z".to_owned()),
        category: RuntimeEventCategory::TemporaryStorageProvisioning,
        count: Some(4),
        observed_at: Some("2026-09-15T12:30:00Z".to_owned()),
    };
    let incident = new_runtime_incident(&event).unwrap();
    assert_eq!(incident.category, event.category);
    assert_eq!(incident.count, 4);

    let restored = pod_event_from_incident(crate::storage::WorkspaceRuntimeIncident {
        workspace_id: Uuid::now_v7(),
        last_observed_at: incident.last_observed_at,
        category: incident.category,
        observed_at: incident.observed_at,
        count: incident.count,
    });
    assert_eq!(restored, event);
}

#[test]
fn current_scheduler_causes_reach_list_runtime_without_a_persisted_event() {
    let current = PodEvent {
        category: RuntimeEventCategory::Scheduling,
        count: None,
        observed_at: Some("2026-10-18T12:14:00Z".into()),
        last_observed_at: Some("2026-10-18T12:14:00Z".into()),
        reason: Some("FailedScheduling".into()),
        component: None,
        scheduling_blockers: vec![crate::runtime_incidents::SchedulingBlocker::CpuCapacity],
    };
    assert_eq!(
        merge_current_events(Vec::new(), std::slice::from_ref(&current)),
        vec![current.clone()]
    );
    let at = "2026-10-18T12:14:00Z"
        .parse::<k8s_openapi::jiff::Timestamp>()
        .unwrap()
        .as_second();
    let historical = WorkspaceRuntimeIncident {
        workspace_id: Uuid::now_v7(),
        category: RuntimeEventCategory::Scheduling,
        observed_at: at,
        last_observed_at: at,
        count: 7,
    };
    let merged = merge_current_events(vec![historical], &[current]);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].count, Some(7));
    assert_eq!(
        merged[0].scheduling_blockers,
        vec![crate::runtime_incidents::SchedulingBlocker::CpuCapacity]
    );
}
