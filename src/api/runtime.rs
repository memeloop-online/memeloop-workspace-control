use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use k8s_openapi::api::core::v1::{Event, PersistentVolumeClaim, Pod};
use kube::{Api, api::ListParams};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    auth::Permission,
    kubernetes::{OWNER_INSTALLATION_LABEL, WORKSPACE_ID_LABEL},
    quota::Resources,
};

use super::{ApiError, AppState, auth::principal};

mod pod_views;
mod storage_metrics;

use pod_views::{
    newest_events, object_workspace_id, pod_event, pod_metrics, pod_metrics_all, pod_runtime,
};
use storage_metrics::fetch as fetch_storage_metrics;
pub(super) use storage_metrics::{StoragePressure, StorageTelemetry, StorageTelemetryStatus};

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct WorkspaceRuntimeResponse {
    allocated: Resources,
    pvc_capacity: Option<String>,
    storage: StorageTelemetry,
    metrics_available: bool,
    pods: Vec<PodRuntime>,
    metrics: Vec<PodMetric>,
    events: Vec<PodEvent>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct WorkspaceRuntimeEntry {
    workspace_id: Uuid,
    runtime: WorkspaceRuntimeResponse,
}

#[derive(Debug, Deserialize, IntoParams)]
pub(super) struct WorkspaceRuntimeListQuery {
    organization_id: Uuid,
    /// Comma-separated workspace IDs from the currently visible page (maximum 100).
    workspace_ids: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct PodRuntime {
    name: String,
    phase: Option<String>,
    ready: bool,
    restarts: i32,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
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

#[utoipa::path(get, path = "/api/v1/workspace-runtimes", params(WorkspaceRuntimeListQuery), responses((status = 200, body = [WorkspaceRuntimeEntry]), (status = 403, body = super::ErrorEnvelope), (status = 503, body = super::ErrorEnvelope)))]
pub(super) async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<WorkspaceRuntimeListQuery>,
) -> Result<Json<Vec<WorkspaceRuntimeEntry>>, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.allows(Permission::ReadWorkspace, query.organization_id) {
        return Err(ApiError::Forbidden);
    }
    let workspace_ids = parse_workspace_ids(&query.workspace_ids)?;
    let workspaces = state
        .database
        .list_workspaces_by_ids(query.organization_id, &workspace_ids)
        .await?;
    if workspaces.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let client = state
        .kubernetes_client
        .clone()
        .ok_or(ApiError::KubernetesUnavailable)?;
    let kubernetes_request = state
        .observability
        .begin_upstream(crate::observability::UpstreamKind::Kubernetes);
    let selector = format!(
        "{OWNER_INSTALLATION_LABEL}={},{} in ({})",
        state.config.installation_id,
        WORKSPACE_ID_LABEL,
        workspaces
            .iter()
            .map(|workspace| workspace.id.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let pod_list = Api::<Pod>::all(client.clone())
        .list(&ListParams::default().labels(&selector))
        .await
        .map_err(ApiError::Kubernetes)?;
    let pvc_list = Api::<PersistentVolumeClaim>::all(client.clone())
        .list(&ListParams::default().labels(&selector))
        .await
        .map_err(ApiError::Kubernetes)?;
    let metric_result = pod_metrics_all(client, &selector).await;
    let metrics_available = metric_result.is_ok();
    let metric_map = metric_result.unwrap_or_else(|error| {
        tracing::debug!(%error, "metrics.k8s.io is unavailable");
        BTreeMap::new()
    });
    kubernetes_request.success();
    let storage_metrics = fetch_storage_metrics(
        state.config.prometheus_url.as_ref(),
        &state.config.installation_id,
        &state.observability,
    )
    .await;
    let observed_now = unix_timestamp();
    let mut pod_map = BTreeMap::<Uuid, Vec<PodRuntime>>::new();
    for pod in &pod_list.items {
        if let Some(workspace_id) = object_workspace_id(&pod.metadata.labels) {
            pod_map
                .entry(workspace_id)
                .or_default()
                .push(pod_runtime(pod));
        }
    }
    let mut pvc_map = BTreeMap::<Uuid, String>::new();
    for pvc in pvc_list.items {
        if let (Some(workspace_id), Some(capacity)) = (
            object_workspace_id(&pvc.metadata.labels),
            pvc.status
                .and_then(|status| status.capacity)
                .and_then(|capacity| capacity.get("storage").map(|value| value.0.clone())),
        ) {
            pvc_map.insert(workspace_id, capacity);
        }
    }
    let response = workspaces
        .into_iter()
        .map(|workspace| {
            let workspace_id = workspace.id;
            let namespace = state
                .config
                .installation_id
                .workspace_namespace(&workspace.short_id)
                .unwrap_or_default();
            WorkspaceRuntimeEntry {
                workspace_id,
                runtime: WorkspaceRuntimeResponse {
                    allocated: workspace.template.resources,
                    pvc_capacity: pvc_map.remove(&workspace_id),
                    storage: storage_metrics.telemetry(&namespace, observed_now),
                    metrics_available,
                    pods: pod_map.remove(&workspace_id).unwrap_or_default(),
                    metrics: metric_map.get(&workspace_id).cloned().unwrap_or_default(),
                    events: Vec::new(),
                },
            }
        })
        .collect();
    Ok(Json(response))
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
    let kubernetes_request = state
        .observability
        .begin_upstream(crate::observability::UpstreamKind::Kubernetes);
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
    kubernetes_request.success();
    let storage = fetch_storage_metrics(
        state.config.prometheus_url.as_ref(),
        &state.config.installation_id,
        &state.observability,
    )
    .await
    .telemetry(&namespace, unix_timestamp());
    let response = WorkspaceRuntimeResponse {
        allocated: workspace.template.resources,
        pvc_capacity,
        storage,
        metrics_available,
        pods,
        metrics,
        events,
    };
    Ok(Json(response))
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64)
}

fn parse_workspace_ids(value: &str) -> Result<Vec<Uuid>, ApiError> {
    let mut ids = value
        .split(',')
        .filter(|value| !value.is_empty())
        .map(Uuid::parse_str)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ApiError::BadRequest("workspace_ids must contain UUIDs"))?;
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() || ids.len() > 100 {
        return Err(ApiError::BadRequest(
            "workspace_ids must contain between 1 and 100 UUIDs",
        ));
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
