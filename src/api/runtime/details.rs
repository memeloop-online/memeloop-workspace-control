use super::*;

#[derive(Debug, Clone, Default)]
pub(super) struct WorkspaceStoragePvcIdentities {
    pub(super) persistent: Option<StorageIdentity>,
    pub(super) temporary: Option<StorageIdentity>,
}

impl WorkspaceStoragePvcIdentities {
    pub(super) fn identities(&self) -> impl Iterator<Item = StorageIdentity> + '_ {
        self.persistent.iter().chain(self.temporary.iter()).cloned()
    }
}

pub(super) struct WorkspaceRuntimeDetails {
    pub(super) storage_pvcs: WorkspaceStoragePvcIdentities,
    pub(super) scratch_backing: StorageBacking,
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

    let storage_pvcs = Api::<PersistentVolumeClaim>::namespaced(client.clone(), namespace)
        .list(&ListParams::default().labels(selector))
        .await
        .map_err(ApiError::Kubernetes)?
        .items;
    let storage_pvcs = storage_pvc_identities(storage_pvcs, workspace_id, installation_id);

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
        storage_pvcs,
        scratch_backing: scratch_backing_from_pods(&pod_list.items),
        metrics_available,
        pods,
        metrics,
        events,
    })
}

fn storage_pvc_identities(
    pvcs: Vec<PersistentVolumeClaim>,
    workspace_id: Uuid,
    installation_id: &str,
) -> WorkspaceStoragePvcIdentities {
    let mut identities = WorkspaceStoragePvcIdentities::default();
    for pvc in pvcs {
        let labels = pvc.metadata.labels.as_ref();
        if object_workspace_id(&pvc.metadata.labels) != Some(workspace_id)
            || !labels.is_some_and(|labels| {
                labels
                    .get(OWNER_INSTALLATION_LABEL)
                    .is_some_and(|value| value == installation_id)
            })
        {
            continue;
        }
        let Some(namespace) = pvc.metadata.namespace else {
            continue;
        };
        let Some(name) = pvc.metadata.name else {
            continue;
        };
        match labels
            .and_then(|labels| labels.get(STORAGE_ROLE_LABEL))
            .map(String::as_str)
        {
            Some(STORAGE_ROLE_HOME) => identities.persistent = Some((namespace, name)),
            Some(STORAGE_ROLE_TEMPORARY) => identities.temporary = Some((namespace, name)),
            _ => {}
        }
    }
    identities
}

pub(super) fn scratch_backing_from_pods(pods: &[Pod]) -> StorageBacking {
    let Some(volume) = pods.iter().find_map(|pod| {
        pod.spec
            .as_ref()?
            .volumes
            .as_ref()?
            .iter()
            .find(|volume| volume.name == "workspace-scratch")
    }) else {
        return StorageBacking::Unknown;
    };
    if volume.ephemeral.is_some() {
        StorageBacking::EphemeralVolume
    } else if volume.empty_dir.is_some() {
        StorageBacking::NodeLocal
    } else {
        StorageBacking::Unknown
    }
}
