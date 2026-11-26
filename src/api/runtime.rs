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
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Pod};
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
    storage::WorkspaceRuntimeIncident,
    workspaces::Workspace,
};

use super::{ApiError, AppState, auth::principal};

mod details;
mod memory_metrics;
pub(super) mod organization_metrics;
pub(super) mod pod_metrics;
mod pod_views;
mod startup;
pub(super) mod storage_metrics;

use details::{
    WorkspaceStoragePvcIdentities, fetch_workspace_runtime_details, scratch_backing_from_pods,
};
use pod_metrics::observed_pod_metrics;
use pod_views::{
    has_live_runtime, is_active_pod, newest_events, object_workspace_id, pod_runtime,
    pod_scheduling_event,
};
pub(super) use storage_metrics::{
    StorageBacking, StoragePressure, StorageTelemetry, StorageTelemetryCoverage,
};
use storage_metrics::{StorageIdentity, StorageMetricBatch};

pub(super) const STORAGE_ROLE_HOME: &str = "home";
pub(super) const STORAGE_ROLE_TEMPORARY: &str = "temporary";

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct WorkspaceRuntimeResponse {
    allocated: Resources,
    persistent_storage: StorageTelemetry,
    temporary_storage: StorageTelemetry,
    /// Whether the workspace has fresh CPU or memory samples for an active Pod.
    metrics_available: bool,
    /// The node hosting the workspace's active Pod, when it has been scheduled.
    node_name: Option<String>,
    /// Last failure for the current requested memory limit; populated on the workspace detail endpoint.
    memory_resize_failure: Option<String>,
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
    startup: startup::PodStartup,
    containers: Vec<startup::ContainerRuntime>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(super) struct PodMetric {
    pod: String,
    container: String,
    cpu: Option<String>,
    memory: Option<String>,
    /// Actual live container limit, which may differ from the workspace template.
    memory_limit: Option<String>,
    /// Metrics-server working-set usage relative to the effective limit.
    /// Missing or stale usage is unknown; a low value does not rule out PSI stalls.
    memory_used_percent: Option<f64>,
    observed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub(super) struct PodEvent {
    id: String,
    details: crate::runtime_incidents::RuntimeEventDetails,
    category: RuntimeEventCategory,
    count: Option<i32>,
    observed_at: Option<String>,
    last_observed_at: Option<String>,
}

type PodRuntimeMap = BTreeMap<Uuid, Vec<PodRuntime>>;
type PodMetricMap = BTreeMap<Uuid, Vec<PodMetric>>;
type WorkspaceStoragePvcMap = BTreeMap<Uuid, WorkspaceStoragePvcIdentities>;
type ScratchBackingMap = BTreeMap<Uuid, StorageBacking>;
type WorkspaceNodeNameMap = BTreeMap<Uuid, String>;

struct KubernetesRuntimeBatch {
    pods: PodRuntimeMap,
    node_names: WorkspaceNodeNameMap,
    storage_pvcs: WorkspaceStoragePvcMap,
    scratch_backings: ScratchBackingMap,
    metrics: PodMetricMap,
    scheduling_events: BTreeMap<Uuid, Vec<PodEvent>>,
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
    let mut workspaces = state
        .database
        .list_workspaces_by_ids(query.organization_id, &workspace_ids)
        .await?;
    workspaces.retain(|workspace| actor.may_access_workspace_template(workspace.template_id));
    if workspaces.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let client = state
        .kubernetes_client
        .clone()
        .ok_or(ApiError::KubernetesUnavailable)?;
    let selector = runtime_selector(&state.config.installation_id, &workspaces);
    let metrics = state.pod_metrics.snapshot();
    let kubernetes_runtime = fetch_kubernetes_runtime(
        &state.observability,
        client,
        &selector,
        &workspaces,
        metrics.as_ref().map(|latest| &latest.metrics),
    )
    .await?;
    let storage_metrics = state.storage_metrics.snapshot();
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
    observability: &crate::observability::Observability,
    client: Client,
    selector: &str,
    workspaces: &[Workspace],
    observed_metrics: Option<&PodMetricMap>,
) -> Result<KubernetesRuntimeBatch, ApiError> {
    let request = observability.begin_upstream(crate::observability::UpstreamKind::Kubernetes);
    let pods = Api::<Pod>::all(client.clone());
    let pvcs = Api::<PersistentVolumeClaim>::all(client.clone());
    let params = ListParams::default().labels(selector);
    let (pod_list, pvc_list) =
        tokio::try_join!(pods.list(&params), pvcs.list(&params)).map_err(ApiError::Kubernetes)?;
    let bound_homes = futures::future::try_join_all(workspaces.iter().filter_map(|workspace| {
        let client = &client;
        let claims = &pvc_list.items;
        workspace
            .home_volume_binding
            .as_ref()
            .map(|binding| async move {
                Ok::<_, ApiError>((
                    workspace.id,
                    details::bound_home_identity(client, claims, binding).await?,
                ))
            })
    }))
    .await?;
    let mut storage_pvcs = index_storage_pvcs(pvc_list.items);
    for (id, identity) in bound_homes {
        storage_pvcs.entry(id).or_default().persistent = identity;
    }
    let now = unix_timestamp();
    let metrics = workspaces
        .iter()
        .filter_map(|workspace| {
            let samples = observed_metrics?.get(&workspace.id)?;
            Some((
                workspace.id,
                observed_pod_metrics(samples, &pod_list.items, now),
            ))
        })
        .collect();
    request.success();
    let (pods, node_names, scratch_backings) = index_pods(&pod_list.items);
    let mut scheduling_events = BTreeMap::<Uuid, Vec<PodEvent>>::new();
    for pod in &pod_list.items {
        if let (Some(id), Some(event)) = (
            object_workspace_id(&pod.metadata.labels),
            pod_scheduling_event(pod),
        ) {
            scheduling_events.entry(id).or_default().push(event);
        }
    }
    Ok(KubernetesRuntimeBatch {
        pods,
        node_names,
        storage_pvcs,
        scratch_backings,
        metrics,
        scheduling_events,
    })
}

fn index_pods(pods: &[Pod]) -> (PodRuntimeMap, WorkspaceNodeNameMap, ScratchBackingMap) {
    let mut runtimes = PodRuntimeMap::new();
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
            runtimes
                .entry(workspace_id)
                .or_default()
                .push(pod_runtime(pod));
        }
    }
    (runtimes, node_names, scratch_backings)
}

fn index_storage_pvcs(pvcs: Vec<PersistentVolumeClaim>) -> WorkspaceStoragePvcMap {
    let mut identities = WorkspaceStoragePvcMap::new();
    for pvc in pvcs {
        let Some(workspace_id) = object_workspace_id(&pvc.metadata.labels) else {
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
            Some(STORAGE_ROLE_HOME) => entry.persistent = StorageIdentity::from_claim(&pvc),
            Some(STORAGE_ROLE_TEMPORARY) => entry.temporary = StorageIdentity::from_claim(&pvc),
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
            let (pods, metrics) = if show_runtime {
                (
                    kubernetes.pods.remove(&workspace_id).unwrap_or_default(),
                    kubernetes.metrics.remove(&workspace_id).unwrap_or_default(),
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
                    metrics_available: !metrics.is_empty(),
                    node_name,
                    memory_resize_failure: None,
                    pods,
                    metrics,
                    events: merge_current_events(
                        incidents.remove(&workspace_id).unwrap_or_default(),
                        &if show_runtime {
                            kubernetes
                                .scheduling_events
                                .remove(&workspace_id)
                                .unwrap_or_default()
                        } else {
                            Vec::new()
                        },
                    ),
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
    let selector = format!(
        "{OWNER_INSTALLATION_LABEL}={},{WORKSPACE_ID_LABEL}={workspace_id}",
        state.config.installation_id,
    );
    let show_runtime = has_live_runtime(workspace.state);
    let observed_now = unix_timestamp();
    let workspace_ids = [workspace_id];
    let installation_id = state.config.installation_id.to_string();
    let metrics = state.pod_metrics.snapshot();
    let (details, incidents, memory_resize_failure) = tokio::join!(
        fetch_workspace_runtime_details(
            &client,
            namespace,
            &selector,
            &workspace,
            show_runtime,
            &installation_id,
            metrics
                .as_ref()
                .and_then(|latest| latest.metrics.get(&workspace_id))
                .map(Vec::as_slice)
                .unwrap_or_default(),
        ),
        state
            .database
            .list_workspace_runtime_incidents(&workspace_ids, observed_now, 50),
        async {
            if show_runtime && state.workspace_live_memory_resize_supported() {
                state
                    .database
                    .workspace_memory_resize_failure(workspace_id, workspace.generation)
                    .await
            } else {
                Ok(None)
            }
        },
    );
    let details = details?;
    kubernetes_request.success();
    let storage_metrics = state.storage_metrics.snapshot();
    let mut incidents = incidents?;
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
        metrics_available: !details.metrics.is_empty(),
        node_name: details.node_name,
        memory_resize_failure: memory_resize_failure?,
        pods: details.pods,
        metrics: details.metrics,
        events: merge_current_events(
            incidents.remove(&workspace_id).unwrap_or_default(),
            &details.scheduling_events,
        ),
    };
    Ok(Json(response))
}

fn pod_event_from_incident(incident: WorkspaceRuntimeIncident) -> PodEvent {
    PodEvent {
        id: incident.id,
        details: incident.details,
        category: incident.category,
        count: Some(i32::try_from(incident.count).unwrap_or(i32::MAX)),
        last_observed_at: k8s_openapi::jiff::Timestamp::new(incident.last_observed_at, 0)
            .ok()
            .map(|timestamp| timestamp.to_string()),
        observed_at: k8s_openapi::jiff::Timestamp::new(incident.observed_at, 0)
            .ok()
            .map(|timestamp| timestamp.to_string()),
    }
}

fn merge_current_events(
    incidents: Vec<WorkspaceRuntimeIncident>,
    current: &[PodEvent],
) -> Vec<PodEvent> {
    let mut events = incidents
        .into_iter()
        .filter(|event| event.details.is_informative(event.category))
        .map(pod_event_from_incident)
        .collect::<Vec<_>>();
    for live in current
        .iter()
        .filter(|event| event.details.is_informative(event.category))
    {
        if let Some(stored) = events.iter_mut().find(|event| event.id == live.id) {
            let timestamp = |event: &PodEvent| {
                event
                    .last_observed_at
                    .as_deref()
                    .and_then(|value| value.parse::<k8s_openapi::jiff::Timestamp>().ok())
            };
            if timestamp(live) >= timestamp(stored) {
                stored.details = live.details.clone();
                stored.last_observed_at = live.last_observed_at.clone();
            }
            stored.count = stored.count.max(live.count);
        } else {
            events.push(live.clone());
        }
    }
    newest_events(&mut events, 50);
    events
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
