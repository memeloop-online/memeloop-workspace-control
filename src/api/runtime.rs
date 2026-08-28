use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use k8s_openapi::api::core::v1::{Event, PersistentVolumeClaim, Pod};
use kube::{
    Api,
    api::ListParams,
    core::{ApiResource, DynamicObject, GroupVersionKind},
};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{auth::Permission, kubernetes::WORKSPACE_ID_LABEL, quota::Resources};

use super::{ApiError, AppState, auth::principal};

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct WorkspaceRuntimeResponse {
    allocated: Resources,
    pvc_capacity: Option<String>,
    metrics_available: bool,
    pods: Vec<PodRuntime>,
    metrics: Vec<PodMetric>,
    events: Vec<PodEvent>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct PodRuntime {
    name: String,
    phase: Option<String>,
    ready: bool,
    restarts: i32,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct PodMetric {
    pod: String,
    container: String,
    cpu: Option<String>,
    memory: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct PodEvent {
    reason: Option<String>,
    message: Option<String>,
    event_type: Option<String>,
    count: Option<i32>,
    last_timestamp: Option<String>,
}

#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/runtime", params(("workspace_id" = Uuid, Path)), responses((status = 200, body = WorkspaceRuntimeResponse), (status = 403, body = super::ErrorEnvelope), (status = 503, body = super::ErrorEnvelope)))]
pub(super) async fn get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<WorkspaceRuntimeResponse>, ApiError> {
    let actor = principal(&state, &headers).await?;
    let workspace = state.database.get_workspace(workspace_id).await?;
    if !actor.allows(Permission::ReadWorkspace, workspace.organization_id) {
        return Err(ApiError::Forbidden);
    }
    let client = state
        .kubernetes_client
        .clone()
        .ok_or(ApiError::KubernetesUnavailable)?;
    let namespace = state
        .config
        .installation_id
        .workspace_namespace(&workspace.short_id)
        .map_err(|_| ApiError::BadRequest("workspace namespace is invalid"))?;
    let selector = format!("{WORKSPACE_ID_LABEL}={workspace_id}");
    let pod_list = Api::<Pod>::namespaced(client.clone(), &namespace)
        .list(&ListParams::default().labels(&selector))
        .await
        .map_err(ApiError::Kubernetes)?;
    let pods = pod_list.items.iter().map(pod_runtime).collect();
    let event_list = Api::<Event>::namespaced(client.clone(), &namespace)
        .list(&ListParams::default().fields("involvedObject.kind=Pod"))
        .await
        .map_err(ApiError::Kubernetes)?;
    let mut events = event_list
        .items
        .into_iter()
        .map(pod_event)
        .collect::<Vec<_>>();
    newest_events(&mut events, 50);
    let pvc_capacity = Api::<PersistentVolumeClaim>::namespaced(client.clone(), &namespace)
        .get_opt("workspace-data-workspace-0")
        .await
        .map_err(ApiError::Kubernetes)?
        .and_then(|pvc| pvc.status)
        .and_then(|status| status.capacity)
        .and_then(|capacity| capacity.get("storage").map(|quantity| quantity.0.clone()));
    let metric_result = pod_metrics(client, &namespace, &selector).await;
    let (metrics_available, metrics) = match metric_result {
        Ok(metrics) => (true, metrics),
        Err(error) => {
            tracing::debug!(%error, "metrics.k8s.io is unavailable");
            (false, Vec::new())
        }
    };
    Ok(Json(WorkspaceRuntimeResponse {
        allocated: workspace.resources,
        pvc_capacity,
        metrics_available,
        pods,
        metrics,
        events,
    }))
}

fn pod_runtime(pod: &Pod) -> PodRuntime {
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

fn pod_event(event: Event) -> PodEvent {
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

fn newest_events(events: &mut Vec<PodEvent>, limit: usize) {
    events.sort_by(|left, right| right.last_timestamp.cmp(&left.last_timestamp));
    events.truncate(limit);
}

async fn pod_metrics(
    client: kube::Client,
    namespace: &str,
    selector: &str,
) -> Result<Vec<PodMetric>, kube::Error> {
    let resource =
        ApiResource::from_gvk(&GroupVersionKind::gvk("metrics.k8s.io", "v1beta1", "Pod"));
    let list = Api::<DynamicObject>::namespaced_with(client, namespace, &resource)
        .list(&ListParams::default().labels(selector))
        .await?;
    let mut metrics = Vec::new();
    for pod in list.items {
        let pod_name = pod.metadata.name.unwrap_or_default();
        if let Some(containers) = pod
            .data
            .get("containers")
            .and_then(|value| value.as_array())
        {
            for container in containers {
                metrics.push(PodMetric {
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
                });
            }
        }
    }
    Ok(metrics)
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
