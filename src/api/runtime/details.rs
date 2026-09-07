use super::*;

pub(super) struct WorkspaceRuntimeDetails {
    pub(super) pvc_capacity: Option<String>,
    pub(super) metrics_available: bool,
    pub(super) pods: Vec<PodRuntime>,
    pub(super) metrics: Vec<PodMetric>,
    pub(super) events: Vec<PodEvent>,
}

pub(super) async fn fetch_workspace_runtime_details(
    client: &Client,
    namespace: &str,
    names: &crate::workspace_runtime::WorkspaceRuntimeNames,
    selector: &str,
    workspace_id: Uuid,
    show_runtime: bool,
    installation_id: &str,
) -> Result<WorkspaceRuntimeDetails, ApiError> {
    let pod_list = Api::<Pod>::namespaced(client.clone(), namespace)
        .list(&ListParams::default().labels(selector))
        .await
        .map_err(ApiError::Kubernetes)?;
    let active_pod_names = active_pod_names(&pod_list.items);
    let pods = if show_runtime {
        pod_list
            .items
            .iter()
            .filter(|pod| is_active_pod(pod))
            .map(pod_runtime)
            .collect()
    } else {
        Vec::new()
    };

    let event_list = Api::<Event>::namespaced(client.clone(), namespace)
        .list(&ListParams::default().fields(&format!(
            "involvedObject.kind=Pod,involvedObject.name={}",
            names.resources.pod_ordinal_zero(),
        )))
        .await
        .map_err(ApiError::Kubernetes)?;
    let mut events = event_list
        .items
        .into_iter()
        .map(pod_event)
        .collect::<Vec<_>>();
    newest_events(&mut events, 50);

    let pvc_capacity = Api::<PersistentVolumeClaim>::namespaced(client.clone(), namespace)
        .get_opt(&names.resources.data_pvc_ordinal_zero())
        .await
        .map_err(ApiError::Kubernetes)?
        .filter(|pvc| {
            object_workspace_id(&pvc.metadata.labels) == Some(workspace_id)
                && pvc.metadata.labels.as_ref().is_some_and(|labels| {
                    labels
                        .get(OWNER_INSTALLATION_LABEL)
                        .is_some_and(|value| value == installation_id)
                })
        })
        .and_then(|pvc| pvc.status)
        .and_then(|status| status.capacity)
        .and_then(|capacity| capacity.get("storage").map(|quantity| quantity.0.clone()));

    let metric_result = pod_metrics(client.clone(), namespace, selector).await;
    let (metrics_available, metrics) = match metric_result {
        Ok(metrics) => (
            true,
            if show_runtime {
                active_pod_metrics(&metrics, &active_pod_names)
            } else {
                Vec::new()
            },
        ),
        Err(error) => {
            tracing::debug!(%error, "metrics.k8s.io is unavailable");
            (false, Vec::new())
        }
    };

    Ok(WorkspaceRuntimeDetails {
        pvc_capacity,
        metrics_available,
        pods,
        metrics,
        events,
    })
}

pub(super) fn storage_identity(
    installation_id: &InstallationId,
    workspace: &Workspace,
) -> Result<StorageIdentity, crate::workspace_runtime::WorkspaceRuntimeIdentityError> {
    Ok((
        workspace.runtime.namespace.clone(),
        crate::workspace_runtime::WorkspaceRuntimeNames::for_workspace(
            installation_id,
            &workspace.runtime,
            &workspace.short_id,
        )?
        .resources
        .data_pvc_ordinal_zero(),
    ))
}
