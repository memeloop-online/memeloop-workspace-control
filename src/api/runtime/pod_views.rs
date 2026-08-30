use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{Event, Pod};
use kube::{
    Api,
    api::ListParams,
    core::{ApiResource, DynamicObject, GroupVersionKind},
};
use uuid::Uuid;

use crate::kubernetes::WORKSPACE_ID_LABEL;

use super::{PodEvent, PodMetric, PodRuntime};

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
        api::core::v1::{Event, EventSeries},
        apimachinery::pkg::apis::meta::v1::{MicroTime, Time},
        jiff::Timestamp,
    };

    use super::{PodEvent, newest_events, pod_event};

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

    fn pod_event_at(value: &str) -> PodEvent {
        pod_event(Event {
            event_time: Some(MicroTime(timestamp(value))),
            ..Event::default()
        })
    }
}
