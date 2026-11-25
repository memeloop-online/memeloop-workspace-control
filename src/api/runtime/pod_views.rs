use std::collections::{BTreeMap, BTreeSet};

use k8s_openapi::api::core::v1::Pod;
use kube::{
    Api,
    api::ListParams,
    core::{ApiResource, DynamicObject, GroupVersionKind},
};
use uuid::Uuid;

use crate::{kubernetes::WORKSPACE_ID_LABEL, workspaces::WorkspaceState};

use super::RuntimeEventCategory;
use super::{PodEvent, PodMetric, PodRuntime};

/// Returns whether a workspace state can have a live runtime to display.
///
/// Stopped and deletion states deliberately hide any late Kubernetes reads: a
/// StatefulSet may have been scaled down while a terminating Pod or a stale
/// metrics-server sample is still visible for a short period.
pub(super) fn has_live_runtime(state: WorkspaceState) -> bool {
    !matches!(
        state,
        WorkspaceState::Stopped | WorkspaceState::Deleting | WorkspaceState::Deleted
    )
}

/// A Pod is a current runtime observation only while it is neither terminating
/// nor completed. Kubernetes can retain both kinds briefly after a workspace
/// has stopped or restarted.
pub(super) fn is_active_pod(pod: &Pod) -> bool {
    pod.metadata.deletion_timestamp.is_none()
        && !matches!(
            pod.status
                .as_ref()
                .and_then(|status| status.phase.as_deref()),
            Some("Succeeded" | "Failed")
        )
}

pub(super) fn active_pod_names(pods: &[Pod]) -> BTreeSet<String> {
    pods.iter()
        .filter(|pod| is_active_pod(pod))
        .filter_map(|pod| pod.metadata.name.clone())
        .collect()
}

/// Returns the node hosting the first active Pod that has been scheduled.
///
/// A workspace normally has one active Pod (the ordinal-zero StatefulSet
/// member). Pending Pods do not have a node name yet, and completed or
/// terminating Pods must not leak a stale node into the runtime response.
pub(super) fn active_pod_node_name(pods: &[Pod]) -> Option<String> {
    pods.iter()
        .filter(|pod| is_active_pod(pod))
        .find_map(|pod| pod.spec.as_ref()?.node_name.clone())
}

pub(super) fn active_pod_metrics(
    metrics: &[PodMetric],
    active_pod_names: &BTreeSet<String>,
) -> Vec<PodMetric> {
    metrics
        .iter()
        .filter(|metric| active_pod_names.contains(&metric.pod))
        .cloned()
        .collect()
}

pub(super) fn pod_runtime(pod: &Pod) -> PodRuntime {
    let statuses = pod
        .status
        .as_ref()
        .and_then(|status| status.container_statuses.as_ref());
    PodRuntime {
        startup: super::startup::pod_startup(pod),
        containers: statuses
            .into_iter()
            .flatten()
            .map(super::startup::container_runtime)
            .collect(),
        name: pod.metadata.name.clone().unwrap_or_default(),
        phase: pod.status.as_ref().and_then(|status| status.phase.clone()),
        ready: statuses.is_some_and(|statuses| {
            !statuses.is_empty() && statuses.iter().all(|status| status.ready)
        }),
        restarts: statuses
            .map(|statuses| statuses.iter().map(|status| status.restart_count).sum())
            .unwrap_or_default(),
    }
}

/// Pod conditions keep the current scheduling cause visible after Event TTL expiry,
/// without adding per-workspace event requests to the batched runtime endpoint.
pub(super) fn pod_scheduling_event(pod: &Pod) -> Option<PodEvent> {
    if !is_active_pod(pod)
        || pod
            .spec
            .as_ref()
            .is_some_and(|spec| spec.node_name.is_some())
    {
        return None;
    }
    let condition = pod
        .status
        .as_ref()?
        .conditions
        .as_ref()?
        .iter()
        .find(|condition| {
            condition.type_ == "PodScheduled"
                && condition.status == "False"
                && condition.reason.as_deref() == Some("Unschedulable")
        })?;
    let observed_at = condition
        .last_transition_time
        .as_ref()
        .or(pod.metadata.creation_timestamp.as_ref())
        .map(|time| time.0.to_string());
    let pod_uid = pod.metadata.uid.as_deref()?.parse::<Uuid>().ok()?;
    let id = format!(
        "pod-condition:{pod_uid}:PodScheduled:{}",
        observed_at.as_deref()?
    );
    Some(PodEvent {
        id,
        category: RuntimeEventCategory::Scheduling,
        count: None,
        observed_at: observed_at.clone(),
        last_observed_at: observed_at,
        details: crate::runtime_incidents::RuntimeEventDetails {
            pod_uid: Some(pod_uid),
            reason: Some("FailedScheduling".to_owned()),
            scheduling_blockers: crate::runtime_incidents::scheduling_blockers(
                condition.message.as_deref(),
            ),
            ..Default::default()
        },
    })
}

pub(super) fn newest_events(events: &mut Vec<PodEvent>, limit: usize) {
    events.sort_by(|left, right| right.observed_at.cmp(&left.observed_at));
    events.truncate(limit);
}

pub(super) async fn pod_metrics(
    client: kube::Client,
    namespace: &str,
    selector: &str,
) -> Result<Vec<PodMetric>, kube::Error> {
    let resource = pod_metrics_resource();
    let list = Api::<DynamicObject>::namespaced_with(client, namespace, &resource)
        .list(&ListParams::default().labels(selector))
        .await?;
    Ok(list.items.into_iter().flat_map(metrics_from_pod).collect())
}

pub(super) async fn pod_metrics_all(
    client: kube::Client,
    selector: &str,
) -> Result<BTreeMap<Uuid, Vec<PodMetric>>, kube::Error> {
    let resource = pod_metrics_resource();
    let list = Api::<DynamicObject>::all_with(client, &resource)
        .list(&ListParams::default().labels(selector))
        .await?;
    let mut metrics = BTreeMap::<Uuid, Vec<PodMetric>>::new();
    for pod in list.items {
        let Some(workspace_id) = object_workspace_id(&pod.metadata.labels) else {
            continue;
        };
        metrics
            .entry(workspace_id)
            .or_default()
            .extend(metrics_from_pod(pod));
    }
    Ok(metrics)
}

pub(super) fn object_workspace_id(labels: &Option<BTreeMap<String, String>>) -> Option<Uuid> {
    labels
        .as_ref()?
        .get(WORKSPACE_ID_LABEL)?
        .parse::<Uuid>()
        .ok()
}

fn pod_metrics_resource() -> ApiResource {
    ApiResource::from_gvk(&GroupVersionKind::gvk("metrics.k8s.io", "v1beta1", "Pod"))
}

fn metrics_from_pod(pod: DynamicObject) -> Vec<PodMetric> {
    let pod_name = pod.metadata.name.unwrap_or_default();
    let observed_at = pod
        .data
        .get("timestamp")
        .and_then(|value| value.as_str())
        .map(str::to_owned);
    pod.data
        .get("containers")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .map(|container| PodMetric {
            memory_limit: None,
            memory_used_percent: None,
            observed_at: observed_at.clone(),
            pod: pod_name.clone(),
            container: container
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_owned(),
            cpu: container
                .pointer("/usage/cpu")
                .and_then(|value| value.as_str())
                .map(str::to_owned),
            memory: container
                .pointer("/usage/memory")
                .and_then(|value| value.as_str())
                .map(str::to_owned),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use k8s_openapi::{
        api::core::v1::{Pod, PodStatus},
        apimachinery::pkg::apis::meta::v1::ObjectMeta,
        apimachinery::pkg::apis::meta::v1::Time,
        jiff::Timestamp,
    };

    use crate::workspaces::WorkspaceState;

    use super::{
        PodEvent, RuntimeEventCategory, active_pod_metrics, active_pod_names, active_pod_node_name,
        has_live_runtime, newest_events, pod_scheduling_event,
    };
    use crate::api::runtime::PodMetric;

    fn timestamp(value: &str) -> Timestamp {
        value.parse().expect("valid test timestamp")
    }

    #[test]
    fn unschedulable_condition_survives_event_expiry_but_not_scheduling_or_termination() {
        let mut pod: Pod = serde_json::from_value(serde_json::json!({
            "metadata": {"uid":"80000000-0000-4000-8000-000000000001", "creationTimestamp":"2026-10-18T12:13:58Z"},
            "spec": {"containers": []},
            "status": {"phase":"Pending", "conditions":[{
                "type":"PodScheduled", "status":"False", "reason":"Unschedulable",
                "lastTransitionTime":"2026-10-18T12:14:00Z",
                "message":"0/9 nodes are available: 1 Insufficient cpu, 1 node(s) had untolerated taint(s), 7 node(s) didn't match Pod's node affinity/selector. private-host private-pvc"
            }]}
        })).unwrap();
        let event = pod_scheduling_event(&pod).expect("current Pending cause");
        assert_eq!(event.category, RuntimeEventCategory::Scheduling);
        assert_eq!(event.observed_at.as_deref(), Some("2026-10-18T12:14:00Z"));
        assert_eq!(
            event.details.scheduling_blockers,
            vec![
                crate::runtime_incidents::SchedulingBlocker::CpuCapacity,
                crate::runtime_incidents::SchedulingBlocker::NodeAffinity,
                crate::runtime_incidents::SchedulingBlocker::Taint,
            ]
        );
        assert!(!serde_json::to_string(&event).unwrap().contains("private"));
        pod.spec.as_mut().unwrap().node_name = Some("private-host".into());
        assert!(pod_scheduling_event(&pod).is_none());
        pod.spec.as_mut().unwrap().node_name = None;
        pod.metadata.deletion_timestamp = Some(Time(timestamp("2026-10-18T12:15:00Z")));
        assert!(pod_scheduling_event(&pod).is_none());
    }

    #[test]
    fn events_are_sorted_before_the_limit_is_applied() {
        let mut events = vec![
            pod_event_at("2026-08-28T08:00:00Z"),
            pod_event_at("2026-08-28T10:00:00Z"),
            pod_event_at("2026-08-28T09:00:00Z"),
        ];
        newest_events(&mut events, 2);
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].observed_at.as_deref(),
            Some("2026-08-28T10:00:00Z")
        );
        assert_eq!(
            events[1].observed_at.as_deref(),
            Some("2026-08-28T09:00:00Z")
        );
    }

    #[test]
    fn stopped_and_deleting_workspaces_hide_runtime_observations() {
        for state in [
            WorkspaceState::Stopped,
            WorkspaceState::Deleting,
            WorkspaceState::Deleted,
        ] {
            assert!(!has_live_runtime(state));
        }
        for state in [
            WorkspaceState::Provisioning,
            WorkspaceState::Ready,
            WorkspaceState::Stopping,
            WorkspaceState::Starting,
            WorkspaceState::Restarting,
            WorkspaceState::Failed,
        ] {
            assert!(has_live_runtime(state));
        }
    }

    #[test]
    fn completed_and_terminating_pods_do_not_supply_runtime_metrics() {
        let running = pod("running", Some("Running"), false);
        let completed = pod("completed", Some("Succeeded"), false);
        let failed = pod("failed", Some("Failed"), false);
        let terminating = pod("terminating", Some("Running"), true);
        let active = active_pod_names(&[running, completed, failed, terminating]);
        assert_eq!(active.into_iter().collect::<Vec<_>>(), vec!["running"]);

        let metrics = active_pod_metrics(
            &[
                metric("running"),
                metric("completed"),
                metric("terminating"),
            ],
            &active_pod_names(&[
                pod("running", Some("Running"), false),
                pod("completed", Some("Succeeded"), false),
                pod("terminating", Some("Running"), true),
            ]),
        );
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].pod, "running");
    }

    #[test]
    fn only_scheduled_active_pods_supply_a_node_name() {
        let mut running = pod("running", Some("Running"), false);
        running.spec = Some(k8s_openapi::api::core::v1::PodSpec {
            node_name: Some("worker-a".to_owned()),
            ..k8s_openapi::api::core::v1::PodSpec::default()
        });
        let mut completed = pod("completed", Some("Succeeded"), false);
        completed.spec = Some(k8s_openapi::api::core::v1::PodSpec {
            node_name: Some("stale-worker".to_owned()),
            ..k8s_openapi::api::core::v1::PodSpec::default()
        });
        let mut pending = pod("pending", Some("Pending"), false);
        pending.spec = Some(k8s_openapi::api::core::v1::PodSpec::default());

        assert_eq!(
            active_pod_node_name(&[completed, pending, running]),
            Some("worker-a".to_owned())
        );
    }

    fn pod_event_at(value: &str) -> PodEvent {
        PodEvent {
            id: value.into(),
            category: RuntimeEventCategory::Other,
            count: None,
            observed_at: Some(value.into()),
            last_observed_at: None,
            details: Default::default(),
        }
    }

    fn pod(name: &str, phase: Option<&str>, terminating: bool) -> Pod {
        Pod {
            metadata: ObjectMeta {
                name: Some(name.to_owned()),
                deletion_timestamp: terminating.then(|| Time(timestamp("2026-08-28T10:00:00Z"))),
                ..ObjectMeta::default()
            },
            status: Some(PodStatus {
                phase: phase.map(str::to_owned),
                ..PodStatus::default()
            }),
            ..Pod::default()
        }
    }

    fn metric(pod: &str) -> PodMetric {
        PodMetric {
            pod: pod.to_owned(),
            container: "workspace".to_owned(),
            cpu: Some("1m".to_owned()),
            memory: Some("1Mi".to_owned()),
            memory_limit: None,
            memory_used_percent: None,
            observed_at: None,
        }
    }
}
