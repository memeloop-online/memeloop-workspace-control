use std::collections::{BTreeMap, BTreeSet};

use k8s_openapi::api::core::v1::{Event, Pod};
use kube::{
    Api,
    api::ListParams,
    core::{ApiResource, DynamicObject, GroupVersionKind},
};
use uuid::Uuid;

use crate::{kubernetes::WORKSPACE_ID_LABEL, workspaces::WorkspaceState};

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

pub(super) fn pod_event(event: Event) -> PodEvent {
    let observed_at = event
        .series
        .as_ref()
        .and_then(|series| series.last_observed_time.as_ref())
        .map(|time| time.0.to_string())
        .or_else(|| event.event_time.as_ref().map(|time| time.0.to_string()))
        .or_else(|| event.last_timestamp.as_ref().map(|time| time.0.to_string()))
        .or_else(|| {
            event
                .metadata
                .creation_timestamp
                .as_ref()
                .map(|time| time.0.to_string())
        });
    let count = event
        .series
        .as_ref()
        .and_then(|series| series.count)
        .or(event.count);
    PodEvent {
        reason: event.reason,
        message: event.message,
        event_type: event.type_,
        count,
        last_timestamp: observed_at,
    }
}

pub(super) fn newest_events(events: &mut Vec<PodEvent>, limit: usize) {
    events.sort_by(|left, right| right.last_timestamp.cmp(&left.last_timestamp));
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
    pod.data
        .get("containers")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .map(|container| PodMetric {
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
        api::core::v1::{Event, EventSeries, Pod, PodStatus},
        apimachinery::pkg::apis::meta::v1::ObjectMeta,
        apimachinery::pkg::apis::meta::v1::{MicroTime, Time},
        jiff::Timestamp,
    };

    use crate::workspaces::WorkspaceState;

    use super::{
        PodEvent, active_pod_metrics, active_pod_names, has_live_runtime, newest_events, pod_event,
    };
    use crate::api::runtime::PodMetric;

    fn timestamp(value: &str) -> Timestamp {
        value.parse().expect("valid test timestamp")
    }

    #[test]
    fn event_prefers_series_time_and_count() {
        let event = Event {
            count: Some(2),
            event_time: Some(MicroTime(timestamp("2026-08-28T09:00:00Z"))),
            last_timestamp: Some(Time(timestamp("2026-08-28T08:00:00Z"))),
            series: Some(EventSeries {
                count: Some(7),
                last_observed_time: Some(MicroTime(timestamp("2026-08-28T10:00:00Z"))),
            }),
            ..Event::default()
        };
        let event = pod_event(event);
        assert_eq!(event.count, Some(7));
        assert_eq!(
            event.last_timestamp.as_deref(),
            Some("2026-08-28T10:00:00Z")
        );
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
            events[0].last_timestamp.as_deref(),
            Some("2026-08-28T10:00:00Z")
        );
        assert_eq!(
            events[1].last_timestamp.as_deref(),
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

    fn pod_event_at(value: &str) -> PodEvent {
        pod_event(Event {
            event_time: Some(MicroTime(timestamp(value))),
            ..Event::default()
        })
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
        }
    }
}
