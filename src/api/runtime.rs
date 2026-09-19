use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use k8s_openapi::api::core::v1::{Event, PersistentVolumeClaim, Pod};
use kube::{Api, Client, api::ListParams};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

pub(super) use crate::storage::RuntimeEventCategory;
use crate::{
    auth::Permission,
    config::InstallationId,
    kubernetes::{OWNER_INSTALLATION_LABEL, STORAGE_ROLE_LABEL, WORKSPACE_ID_LABEL},
    quota::Resources,
    storage::{NewWorkspaceRuntimeIncident, WorkspaceRuntimeIncident},
    workspaces::Workspace,
};

use super::{ApiError, AppState, auth::principal};

mod details;
pub(super) mod organization_metrics;
mod pod_views;
mod storage_metrics;

use details::{
    WorkspaceStoragePvcIdentities, fetch_workspace_runtime_details, scratch_backing_from_pods,
};
use pod_views::{
    active_pod_metrics, active_pod_names, active_pod_node_name, has_live_runtime, is_active_pod,
    newest_events, object_workspace_id, pod_event, pod_metrics, pod_metrics_all, pod_runtime,
};
pub(super) use storage_metrics::{
    StorageBacking, StoragePressure, StorageTelemetry, StorageTelemetryCoverage,
};
use storage_metrics::{StorageIdentity, StorageMetricBatch, fetch as fetch_storage_metrics};

pub(super) const STORAGE_ROLE_HOME: &str = "home";
pub(super) const STORAGE_ROLE_TEMPORARY: &str = "temporary";

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct WorkspaceRuntimeResponse {
    allocated: Resources,
    persistent_storage: StorageTelemetry,
    temporary_storage: StorageTelemetry,
    metrics_available: bool,
    /// The node hosting the workspace's active Pod, when it has been scheduled.
    node_name: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub(super) struct PodEvent {
    category: RuntimeEventCategory,
    count: Option<i32>,
    observed_at: Option<String>,
}

type PodRuntimeMap = BTreeMap<Uuid, Vec<PodRuntime>>;
type ActivePodMap = BTreeMap<Uuid, BTreeSet<String>>;
type PodMetricMap = BTreeMap<Uuid, Vec<PodMetric>>;
type WorkspaceStoragePvcMap = BTreeMap<Uuid, WorkspaceStoragePvcIdentities>;
type ScratchBackingMap = BTreeMap<Uuid, StorageBacking>;
type WorkspaceNodeNameMap = BTreeMap<Uuid, String>;

struct KubernetesRuntimeBatch {
    pods: PodRuntimeMap,
    active_pods: ActivePodMap,
    node_names: WorkspaceNodeNameMap,
    storage_pvcs: WorkspaceStoragePvcMap,
    scratch_backings: ScratchBackingMap,
    metrics: PodMetricMap,
    metrics_available: bool,
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
    let selector = runtime_selector(&state.config.installation_id, &workspaces);
    let kubernetes_runtime = fetch_kubernetes_runtime(state.as_ref(), client, &selector).await?;
    let storage_identities = kubernetes_runtime
        .storage_pvcs
        .values()
        .flat_map(WorkspaceStoragePvcIdentities::identities)
        .collect::<Vec<_>>();
    let storage_metrics = fetch_storage_metrics(
        state.config.prometheus_url.as_ref(),
        &storage_identities,
        &state.observability,
    )
    .await;
    let observed_now = unix_timestamp();
    let workspace_ids = workspaces
        .iter()
        .map(|workspace| workspace.id)
        .collect::<Vec<_>>();
    let incidents = state
        .database
        .list_workspace_runtime_incidents(&workspace_ids, observed_now, 50)
        .await?;
    Ok(Json(build_runtime_entries(
        workspaces,
        kubernetes_runtime,
        &storage_metrics,
        incidents,
        observed_now,
    )))
}

fn runtime_selector(installation_id: &InstallationId, workspaces: &[Workspace]) -> String {
    let workspace_ids = workspaces
        .iter()
        .map(|workspace| workspace.id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{OWNER_INSTALLATION_LABEL}={installation_id},{WORKSPACE_ID_LABEL} in ({workspace_ids})"
    )
}

async fn fetch_kubernetes_runtime(
    state: &AppState,
    client: Client,
    selector: &str,
) -> Result<KubernetesRuntimeBatch, ApiError> {
    let request = state
        .observability
        .begin_upstream(crate::observability::UpstreamKind::Kubernetes);
    let pod_list = Api::<Pod>::all(client.clone())
        .list(&ListParams::default().labels(selector))
        .await
        .map_err(ApiError::Kubernetes)?;
    let pvc_list = Api::<PersistentVolumeClaim>::all(client.clone())
        .list(&ListParams::default().labels(selector))
        .await
        .map_err(ApiError::Kubernetes)?;
    let metric_result = pod_metrics_all(client, selector).await;
    let metrics_available = metric_result.is_ok();
    let metrics = metric_result.unwrap_or_else(|error| {
        tracing::debug!(%error, "metrics.k8s.io is unavailable");
        BTreeMap::new()
    });
    request.success();
    let (pods, active_pods, node_names, scratch_backings) = index_pods(&pod_list.items);
    Ok(KubernetesRuntimeBatch {
        pods,
        active_pods,
        node_names,
        storage_pvcs: index_storage_pvcs(pvc_list.items),
        scratch_backings,
        metrics,
        metrics_available,
    })
}

fn index_pods(
    pods: &[Pod],
) -> (
    PodRuntimeMap,
    ActivePodMap,
    WorkspaceNodeNameMap,
    ScratchBackingMap,
) {
    let mut runtimes = PodRuntimeMap::new();
    let mut active_names = ActivePodMap::new();
    let mut node_names = WorkspaceNodeNameMap::new();
    let mut scratch_backings = ScratchBackingMap::new();
    for pod in pods {
        let Some(workspace_id) = object_workspace_id(&pod.metadata.labels) else {
            continue;
        };
        scratch_backings
            .entry(workspace_id)
            .or_insert_with(|| scratch_backing_from_pods(std::slice::from_ref(pod)));
        if is_active_pod(pod) {
            if let Some(node_name) = pod.spec.as_ref().and_then(|spec| spec.node_name.as_ref()) {
                node_names
                    .entry(workspace_id)
                    .or_insert_with(|| node_name.clone());
            }
            if let Some(name) = &pod.metadata.name {
                active_names
                    .entry(workspace_id)
                    .or_default()
                    .insert(name.clone());
            }
            runtimes
                .entry(workspace_id)
                .or_default()
                .push(pod_runtime(pod));
        }
    }
    (runtimes, active_names, node_names, scratch_backings)
}

fn index_storage_pvcs(pvcs: Vec<PersistentVolumeClaim>) -> WorkspaceStoragePvcMap {
    let mut identities = WorkspaceStoragePvcMap::new();
    for pvc in pvcs {
        let Some(workspace_id) = object_workspace_id(&pvc.metadata.labels) else {
            continue;
        };
        let Some(name) = pvc.metadata.name else {
            continue;
        };
        let Some(namespace) = pvc.metadata.namespace else {
            continue;
        };
        let role = pvc
            .metadata
            .labels
            .as_ref()
            .and_then(|labels| labels.get(STORAGE_ROLE_LABEL))
            .map(String::as_str);
        let entry = identities.entry(workspace_id).or_default();
        match role {
            Some(STORAGE_ROLE_HOME) => entry.persistent = Some((namespace, name)),
            Some(STORAGE_ROLE_TEMPORARY) => entry.temporary = Some((namespace, name)),
            _ => {}
        }
    }
    identities
}

fn build_runtime_entries(
    workspaces: Vec<Workspace>,
    mut kubernetes: KubernetesRuntimeBatch,
    storage_metrics: &StorageMetricBatch,
    mut incidents: BTreeMap<Uuid, Vec<WorkspaceRuntimeIncident>>,
    observed_now: i64,
) -> Vec<WorkspaceRuntimeEntry> {
    workspaces
        .into_iter()
        .map(|workspace| {
            let workspace_id = workspace.id;
            let show_runtime = has_live_runtime(workspace.state);
            let node_name = show_runtime
                .then(|| kubernetes.node_names.remove(&workspace_id))
                .flatten();
            let active_pod_names = kubernetes
                .active_pods
                .remove(&workspace_id)
                .unwrap_or_default();
            let (pods, metrics) = if show_runtime {
                (
                    kubernetes.pods.remove(&workspace_id).unwrap_or_default(),
                    active_pod_metrics(
                        kubernetes
                            .metrics
                            .get(&workspace_id)
                            .map(Vec::as_slice)
                            .unwrap_or_default(),
                        &active_pod_names,
                    ),
                )
            } else {
                (Vec::new(), Vec::new())
            };
            let storage_pvcs = kubernetes
                .storage_pvcs
                .remove(&workspace_id)
                .unwrap_or_default();
            let scratch_backing = if storage_pvcs.temporary.is_some() {
                StorageBacking::EphemeralVolume
            } else {
                kubernetes
                    .scratch_backings
                    .remove(&workspace_id)
                    .unwrap_or(StorageBacking::Unknown)
            };
            WorkspaceRuntimeEntry {
                workspace_id,
                runtime: WorkspaceRuntimeResponse {
                    allocated: workspace.template.resources,
                    persistent_storage: storage_metrics.telemetry(
                        storage_pvcs.persistent.as_ref(),
                        gibibytes(workspace.template.resources.disk_gib),
                        StorageBacking::PersistentVolume,
                        observed_now,
                    ),
                    temporary_storage: storage_metrics.telemetry(
                        storage_pvcs.temporary.as_ref(),
                        gibibytes(workspace.template.storage_policy.temporary_storage_gib),
                        scratch_backing,
                        observed_now,
                    ),
                    metrics_available: kubernetes.metrics_available,
                    node_name,
                    pods,
                    metrics,
                    events: incidents
                        .remove(&workspace_id)
                        .unwrap_or_default()
                        .into_iter()
                        .map(pod_event_from_incident)
                        .collect(),
                },
            }
        })
        .collect()
}

#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/runtime", params(("workspace_id" = Uuid, Path)), responses((status = 200, body = WorkspaceRuntimeResponse), (status = 403, body = super::ErrorEnvelope), (status = 503, body = super::ErrorEnvelope)))]
pub(super) async fn get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<WorkspaceRuntimeResponse>, ApiError> {
    let actor = principal(&state, &headers).await?;
    let workspace = state.database.get_workspace(workspace_id).await?;
    if !actor.allows(Permission::ReadWorkspace, workspace.organization_id)
        || !actor.may_access_workspace_template(workspace.template_id)
    {
        return Err(ApiError::Forbidden);
    }
    let client = state
        .kubernetes_client
        .clone()
        .ok_or(ApiError::KubernetesUnavailable)?;
    let kubernetes_request = state
        .observability
        .begin_upstream(crate::observability::UpstreamKind::Kubernetes);
    let namespace = workspace.runtime.namespace();
    let names = crate::workspace_runtime::WorkspaceRuntimeNames::for_workspace(
        &state.config.installation_id,
        &workspace.runtime,
        &workspace.short_id,
    )
    .map_err(|_| ApiError::BadRequest("workspace runtime identity is invalid"))?;
    let selector = format!(
        "{OWNER_INSTALLATION_LABEL}={},{WORKSPACE_ID_LABEL}={workspace_id}",
        state.config.installation_id,
    );
    let show_runtime = has_live_runtime(workspace.state);
    let details = fetch_workspace_runtime_details(
        &client,
        namespace,
        &names,
        &selector,
        workspace_id,
        show_runtime,
        &state.config.installation_id.to_string(),
    )
    .await?;
    kubernetes_request.success();
    let storage_metrics = fetch_storage_metrics(
        state.config.prometheus_url.as_ref(),
        &details.storage_pvcs.identities().collect::<Vec<_>>(),
        &state.observability,
    )
    .await;
    let observed_now = unix_timestamp();
    let current_incidents = details
        .events
        .iter()
        .filter_map(new_runtime_incident)
        .collect::<Vec<_>>();
    state
        .database
        .upsert_workspace_runtime_incidents(workspace_id, &current_incidents, observed_now)
        .await?;
    let mut incidents = state
        .database
        .list_workspace_runtime_incidents(&[workspace_id], observed_now, 50)
        .await?;
    let response = WorkspaceRuntimeResponse {
        allocated: workspace.template.resources,
        persistent_storage: storage_metrics.telemetry(
            details.storage_pvcs.persistent.as_ref(),
            gibibytes(workspace.template.resources.disk_gib),
            StorageBacking::PersistentVolume,
            observed_now,
        ),
        temporary_storage: storage_metrics.telemetry(
            details.storage_pvcs.temporary.as_ref(),
            gibibytes(workspace.template.storage_policy.temporary_storage_gib),
            if details.storage_pvcs.temporary.is_some() {
                StorageBacking::EphemeralVolume
            } else {
                details.scratch_backing
            },
            observed_now,
        ),
        metrics_available: details.metrics_available,
        node_name: details.node_name,
        pods: details.pods,
        metrics: details.metrics,
        events: incidents
            .remove(&workspace_id)
            .unwrap_or_default()
            .into_iter()
            .map(pod_event_from_incident)
            .collect(),
    };
    Ok(Json(response))
}

fn new_runtime_incident(event: &PodEvent) -> Option<NewWorkspaceRuntimeIncident> {
    let observed_at = event
        .observed_at
        .as_deref()?
        .parse::<k8s_openapi::jiff::Timestamp>()
        .ok()?
        .as_second();
    Some(NewWorkspaceRuntimeIncident {
        category: event.category,
        observed_at,
        last_observed_at: observed_at,
        count: event
            .count
            .and_then(|count| u32::try_from(count).ok())
            .unwrap_or(1),
    })
}

fn pod_event_from_incident(incident: WorkspaceRuntimeIncident) -> PodEvent {
    PodEvent {
        category: incident.category,
        count: Some(i32::try_from(incident.count).unwrap_or(i32::MAX)),
        observed_at: k8s_openapi::jiff::Timestamp::new(incident.observed_at, 0)
            .ok()
            .map(|timestamp| timestamp.to_string()),
    }
}

fn gibibytes(value: u64) -> u64 {
    value.saturating_mul(1024 * 1024 * 1024)
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
mod tests;
