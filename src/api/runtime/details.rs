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
    pub(super) node_name: Option<String>,
    pub(super) pods: Vec<PodRuntime>,
    pub(super) metrics: Vec<PodMetric>,
    pub(super) events: Vec<PodEvent>,
}

pub(super) async fn fetch_workspace_runtime_details(
    client: &Client,
    namespace: &str,
    names: &crate::workspace_runtime::WorkspaceRuntimeNames,
    selector: &str,
    workspace: &Workspace,
    show_runtime: bool,
    installation_id: &str,
) -> Result<WorkspaceRuntimeDetails, ApiError> {
    let pods_api = Api::<Pod>::namespaced(client.clone(), namespace);
    let events_api = Api::<Event>::namespaced(client.clone(), namespace);
    let params = ListParams::default().labels(selector);
    let event_params = ListParams::default().fields(&format!(
        "involvedObject.kind=Pod,involvedObject.name={},type=Warning",
        names.resources.pod_ordinal_zero(),
    ));
    let storage = async {
        let claims = Api::<PersistentVolumeClaim>::namespaced(client.clone(), namespace)
            .list(&params)
            .await
            .map_err(ApiError::Kubernetes)?
            .items;
        let bound_home = match workspace.home_volume_binding.as_ref() {
            Some(binding) => Some(bound_home_identity(client, &claims, binding).await?),
            None => None,
        };
        let mut identities = storage_pvc_identities(claims, workspace.id, installation_id);
        if let Some(identity) = bound_home {
            identities.persistent = identity;
        }
        Ok::<_, ApiError>(identities)
    };
    let (pod_list, event_list, storage_pvcs, metric_result) = tokio::join!(
        pods_api.list(&params),
        events_api.list(&event_params),
        storage,
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            pod_metrics(client.clone(), namespace, selector),
        ),
    );
    let pod_list = pod_list.map_err(ApiError::Kubernetes)?;
    let event_list = event_list.map_err(ApiError::Kubernetes)?;
    let storage_pvcs = storage_pvcs?;
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

    let mut events = warning_events(event_list.items);
    if show_runtime {
        for event in pod_list.items.iter().filter_map(pod_scheduling_event) {
            if !events.iter().any(|existing| existing.id == event.id) {
                events.push(event);
            }
        }
        newest_events(&mut events, 50);
    }

    let metric_result = metric_result
        .map_err(|error| error.to_string())
        .and_then(|result| result.map_err(|error| error.to_string()));
    let (metrics_available, mut metrics) = match metric_result {
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

    super::memory_metrics::annotate(&mut metrics, &pod_list.items, unix_timestamp());

    Ok(WorkspaceRuntimeDetails {
        storage_pvcs,
        scratch_backing: scratch_backing_from_pods(&pod_list.items),
        metrics_available,
        node_name: show_runtime
            .then(|| active_pod_node_name(&pod_list.items))
            .flatten(),
        pods,
        metrics,
        events,
    })
}

pub(super) async fn bound_home_identity(
    client: &Client,
    claims: &[PersistentVolumeClaim],
    binding: &crate::workspaces::WorkspaceHomeVolumeBinding,
) -> Result<Option<StorageIdentity>, ApiError> {
    if let Some(claim) = claims.iter().find(|claim| {
        claim.metadata.namespace.as_deref() == Some(binding.namespace.as_str())
            && claim.metadata.name.as_deref() == Some(binding.claim_name.as_str())
    }) {
        return Ok(listed_bound_home_identity(
            std::slice::from_ref(claim),
            binding,
        ));
    }
    let claim = Api::<PersistentVolumeClaim>::namespaced(client.clone(), &binding.namespace)
        .get_opt(&binding.claim_name)
        .await
        .map_err(ApiError::Kubernetes)?;
    Ok(claim
        .as_ref()
        .and_then(|claim| listed_bound_home_identity(std::slice::from_ref(claim), binding)))
}

pub(super) fn listed_bound_home_identity(
    claims: &[PersistentVolumeClaim],
    binding: &crate::workspaces::WorkspaceHomeVolumeBinding,
) -> Option<StorageIdentity> {
    claims
        .iter()
        .find(|claim| {
            claim.metadata.namespace.as_deref() == Some(binding.namespace.as_str())
                && claim.metadata.name.as_deref() == Some(binding.claim_name.as_str())
                && claim.metadata.uid.as_deref() == Some(binding.claim_uid.as_str())
                && claim.metadata.deletion_timestamp.is_none()
                && claim
                    .status
                    .as_ref()
                    .and_then(|status| status.phase.as_deref())
                    == Some("Bound")
        })
        .map(|_| (binding.namespace.clone(), binding.claim_name.clone()))
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

fn warning_events(events: Vec<Event>) -> Vec<PodEvent> {
    let mut events = events
        .into_iter()
        .filter(|event| {
            event.type_.as_deref() == Some("Warning")
                && !crate::runtime_incidents::startup_gate_pending(event)
        })
        .filter_map(pod_event)
        .filter(|event| event.details.is_informative(event.category))
        .collect::<Vec<_>>();
    newest_events(&mut events, 50);
    events
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
    } else if volume.persistent_volume_claim.is_some() {
        StorageBacking::PersistentVolume
    } else {
        StorageBacking::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    #[tokio::test]
    async fn runtime_details_read_independently_and_resolve_an_adopted_home() {
        let id = Uuid::now_v7();
        let workspace = Workspace {
            id,
            short_id: crate::workspace_runtime::workspace_short_id_for(id),
            organization_id: Uuid::now_v7(),
            owner_id: Uuid::now_v7(),
            name: "runtime concurrency".into(),
            template_id: None,
            node_pool: "default".into(),
            runtime: crate::workspace_runtime::WorkspaceRuntimeIdentity,
            home_volume_binding: Some(crate::workspaces::WorkspaceHomeVolumeBinding {
                namespace: "workspaces".into(),
                claim_name: "adopted-home".into(),
                claim_uid: "home-uid".into(),
                capacity_gib: 20,
            }),
            scratch_volume_binding: None,
            template: crate::templates::WorkspaceTemplateSpec::standard(
                "registry.example/dev:latest",
                crate::workspaces::AccessMode::Internal,
                crate::quota::Resources {
                    cpu_millis: 1_000,
                    memory_mib: 1_024,
                    gpu_count: 0,
                    disk_gib: 20,
                },
            ),
            state: crate::workspaces::WorkspaceState::Ready,
            generation: 1,
            created_at: 1,
            updated_at: 1,
        };
        let barrier = Arc::new(tokio::sync::Barrier::new(4));
        let paths = Arc::new(std::sync::Mutex::new(Vec::new()));
        let service = tower::service_fn({
            let paths = paths.clone();
            move |request: axum::http::Request<kube::client::Body>| {
                let barrier = barrier.clone();
                let paths = paths.clone();
                async move {
                    let path = request.uri().path().to_owned();
                    paths.lock().unwrap().push(path.clone());
                    let (version, kind) = match path.as_str() {
                        "/api/v1/namespaces/workspaces/pods" => ("v1", "PodList"),
                        "/api/v1/namespaces/workspaces/events" => ("v1", "EventList"),
                        "/api/v1/namespaces/workspaces/persistentvolumeclaims" => {
                            ("v1", "PersistentVolumeClaimList")
                        }
                        "/apis/metrics.k8s.io/v1beta1/namespaces/workspaces/pods" => {
                            ("metrics.k8s.io/v1beta1", "PodMetricsList")
                        }
                        "/api/v1/namespaces/workspaces/persistentvolumeclaims/adopted-home" => {
                            ("v1", "PersistentVolumeClaim")
                        }
                        other => panic!("unexpected runtime request: {other}"),
                    };
                    let body = if kind == "PersistentVolumeClaim" {
                        assert_eq!(paths.lock().unwrap().len(), 5);
                        serde_json::json!({
                            "apiVersion": version, "kind": kind,
                            "metadata": {"namespace":"workspaces", "name":"adopted-home", "uid":"home-uid"},
                            "status": {"phase":"Bound"},
                        })
                    } else {
                        barrier.wait().await;
                        serde_json::json!({"apiVersion":version,"kind":kind,"metadata":{},"items":[]})
                    };
                    Ok::<_, std::convert::Infallible>(axum::http::Response::new(
                        axum::body::Body::from(body.to_string()),
                    ))
                }
            }
        });
        let installation = "test".parse().unwrap();
        let names = crate::workspace_runtime::WorkspaceRuntimeNames::for_workspace(
            &installation,
            &workspace.runtime,
            &workspace.short_id,
        )
        .unwrap();
        let details = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            fetch_workspace_runtime_details(
                &Client::new(service, "default"),
                "workspaces",
                &names,
                "workspace.memeloop.dev/owner-installation=test",
                &workspace,
                true,
                "test",
            ),
        )
        .await
        .expect("independent runtime reads must start together")
        .unwrap();
        assert!(details.metrics_available);
        assert_eq!(
            details.storage_pvcs.persistent,
            Some(("workspaces".into(), "adopted-home".into()))
        );
        assert!(details.storage_pvcs.temporary.is_none());
        assert_eq!(paths.lock().unwrap().len(), 5);
    }

    #[test]
    fn listed_home_uses_exact_bound_claim_instead_of_retained_pre_migration_home() {
        let binding = crate::workspaces::WorkspaceHomeVolumeBinding {
            namespace: "workspaces".into(),
            claim_name: "home-thin".into(),
            claim_uid: "original-home-uid".into(),
            capacity_gib: 20,
        };
        let claim = PersistentVolumeClaim {
            metadata: ObjectMeta {
                namespace: Some(binding.namespace.clone()),
                name: Some(binding.claim_name.clone()),
                uid: Some(binding.claim_uid.clone()),
                ..ObjectMeta::default()
            },
            status: Some(k8s_openapi::api::core::v1::PersistentVolumeClaimStatus {
                phase: Some("Bound".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut retained = claim.clone();
        retained.metadata.name = Some("old-home".into());
        assert_eq!(
            listed_bound_home_identity(&[retained, claim.clone()], &binding),
            Some((binding.namespace.clone(), binding.claim_name.clone()))
        );
        let mut replaced = claim.clone();
        replaced.metadata.uid = Some("replacement-uid".into());
        assert_eq!(listed_bound_home_identity(&[replaced], &binding), None);
        let mut pending = claim.clone();
        pending.status.as_mut().unwrap().phase = Some("Pending".into());
        assert_eq!(listed_bound_home_identity(&[pending], &binding), None);
        let mut terminating = claim;
        terminating.metadata.deletion_timestamp =
            Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(
                "2026-11-02T00:00:00Z".parse().unwrap(),
            ));
        assert_eq!(listed_bound_home_identity(&[terminating], &binding), None);
    }

    #[tokio::test]
    async fn bound_home_only_fetches_claims_absent_from_the_labeled_list() {
        let binding = crate::workspaces::WorkspaceHomeVolumeBinding {
            namespace: "workspaces".into(),
            claim_name: "adopted-home".into(),
            claim_uid: "original-home-uid".into(),
            capacity_gib: 20,
        };
        let claim: PersistentVolumeClaim = serde_json::from_value(serde_json::json!({
            "apiVersion": "v1", "kind": "PersistentVolumeClaim",
            "metadata": {
                "namespace": binding.namespace, "name": binding.claim_name,
                "uid": binding.claim_uid,
            },
            "status": {"phase": "Bound"},
        }))
        .unwrap();
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let service = tower::service_fn({
            let requests = requests.clone();
            let body = serde_json::to_string(&claim).unwrap();
            move |request: axum::http::Request<kube::client::Body>| {
                assert_eq!(request.method(), axum::http::Method::GET);
                assert_eq!(
                    request.uri().path(),
                    "/api/v1/namespaces/workspaces/persistentvolumeclaims/adopted-home"
                );
                requests.lock().unwrap().push(request.uri().to_string());
                let body = body.clone();
                async move {
                    Ok::<_, std::convert::Infallible>(axum::http::Response::new(
                        axum::body::Body::from(body),
                    ))
                }
            }
        });
        let client = Client::new(service, "default");
        let expected = Some((binding.namespace.clone(), binding.claim_name.clone()));
        assert_eq!(
            bound_home_identity(&client, &[], &binding).await.unwrap(),
            expected
        );
        assert_eq!(requests.lock().unwrap().len(), 1);
        assert_eq!(
            bound_home_identity(&client, std::slice::from_ref(&claim), &binding)
                .await
                .unwrap(),
            expected
        );
        assert_eq!(requests.lock().unwrap().len(), 1);
        let mut replaced = claim;
        replaced.metadata.uid = Some("replacement-uid".into());
        assert_eq!(
            bound_home_identity(&client, &[replaced], &binding)
                .await
                .unwrap(),
            None
        );
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn details_only_forward_warning_events_for_persistence() {
        let normal = Event {
            metadata: ObjectMeta {
                uid: Some("b8ac1a24-5da0-4f53-bb76-7c2b92dce026".into()),
                ..ObjectMeta::default()
            },
            type_: Some("Normal".into()),
            reason: Some("SuccessfulAttachVolume".into()),
            message: Some("AttachVolume succeeded for workspace-scratch".into()),
            ..Event::default()
        };
        let warning = Event {
            metadata: ObjectMeta {
                uid: Some("49d98da1-b26a-4cb2-b7a9-c848d5bb6484".into()),
                ..ObjectMeta::default()
            },
            type_: Some("Warning".into()),
            reason: Some("Evicted".into()),
            message: Some("The node was low on resource: ephemeral-storage.".into()),
            ..Event::default()
        };
        let events = warning_events(vec![normal, Event::default(), warning]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].category, RuntimeEventCategory::EphemeralStorage);
    }
}
