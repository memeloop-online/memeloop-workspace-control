use super::{pod_views::active_pod_node_name, *};

#[derive(Debug, Clone, Default)]
pub(super) struct WorkspaceStoragePvcIdentities {
    pub(super) persistent: Option<StorageIdentity>,
    pub(super) temporary: Option<StorageIdentity>,
}

pub(super) struct WorkspaceRuntimeDetails {
    pub(super) storage_pvcs: WorkspaceStoragePvcIdentities,
    pub(super) scratch_backing: StorageBacking,
    pub(super) node_name: Option<String>,
    pub(super) pods: Vec<PodRuntime>,
    pub(super) metrics: Vec<PodMetric>,
    pub(super) scheduling_events: Vec<PodEvent>,
}

pub(super) async fn fetch_workspace_runtime_details(
    client: &Client,
    namespace: &str,
    selector: &str,
    workspace: &Workspace,
    show_runtime: bool,
    installation_id: &str,
    observed_metrics: &[PodMetric],
) -> Result<WorkspaceRuntimeDetails, ApiError> {
    let pods_api = Api::<Pod>::namespaced(client.clone(), namespace);
    let params = ListParams::default().labels(selector);
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
    let (pod_list, storage_pvcs) = tokio::join!(pods_api.list(&params), storage);
    let pod_list = pod_list.map_err(ApiError::Kubernetes)?;
    let storage_pvcs = storage_pvcs?;
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

    let scheduling_events = if show_runtime {
        pod_list
            .items
            .iter()
            .filter_map(pod_scheduling_event)
            .collect()
    } else {
        Vec::new()
    };

    let metrics = if show_runtime {
        observed_pod_metrics(observed_metrics, &pod_list.items, unix_timestamp())
    } else {
        Vec::new()
    };
    Ok(WorkspaceRuntimeDetails {
        storage_pvcs,
        scratch_backing: scratch_backing_from_pods(&pod_list.items),
        node_name: show_runtime
            .then(|| active_pod_node_name(&pod_list.items))
            .flatten(),
        pods,
        metrics,
        scheduling_events,
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
        .and_then(StorageIdentity::from_claim)
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
        match labels
            .and_then(|labels| labels.get(STORAGE_ROLE_LABEL))
            .map(String::as_str)
        {
            Some(STORAGE_ROLE_HOME) => identities.persistent = StorageIdentity::from_claim(&pvc),
            Some(STORAGE_ROLE_TEMPORARY) => {
                identities.temporary = StorageIdentity::from_claim(&pvc)
            }
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
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
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
                        "/api/v1/namespaces/workspaces/persistentvolumeclaims" => {
                            ("v1", "PersistentVolumeClaimList")
                        }
                        "/api/v1/namespaces/workspaces/persistentvolumeclaims/adopted-home" => {
                            ("v1", "PersistentVolumeClaim")
                        }
                        other => panic!("unexpected runtime request: {other}"),
                    };
                    let body = if kind == "PersistentVolumeClaim" {
                        assert_eq!(paths.lock().unwrap().len(), 3);
                        serde_json::json!({
                            "apiVersion": version, "kind": kind,
                            "metadata": {"namespace":"workspaces", "name":"adopted-home", "uid":"home-uid"},
                            "status": {"phase":"Bound"},
                        })
                    } else {
                        barrier.wait().await;
                        let items = if kind == "PodList" {
                            serde_json::json!([{
                                "metadata": {"name": "workspace-0", "uid": "80000000-0000-4000-8000-000000000001"},
                                "spec": {"containers": []},
                                "status": {"phase": "Pending", "conditions": [{
                                    "type": "PodScheduled", "status": "False", "reason": "Unschedulable",
                                    "lastTransitionTime": "2026-10-18T12:14:00Z",
                                    "message": "0/9 nodes are available: 9 Insufficient cpu"
                                }]}
                            }])
                        } else {
                            serde_json::json!([])
                        };
                        serde_json::json!({"apiVersion":version,"kind":kind,"metadata":{},"items":items})
                    };
                    Ok::<_, std::convert::Infallible>(axum::http::Response::new(
                        axum::body::Body::from(body.to_string()),
                    ))
                }
            }
        });
        let details = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            fetch_workspace_runtime_details(
                &Client::new(service, "default"),
                "workspaces",
                "workspace.memeloop.dev/owner-installation=test",
                &workspace,
                true,
                "test",
                &[],
            ),
        )
        .await
        .expect("independent runtime reads must start together")
        .unwrap();
        assert!(details.metrics.is_empty());
        assert_eq!(
            details.storage_pvcs.persistent,
            StorageIdentity::from_claim(&PersistentVolumeClaim {
                metadata: ObjectMeta {
                    namespace: Some("workspaces".into()),
                    name: Some("adopted-home".into()),
                    ..Default::default()
                },
                ..Default::default()
            })
        );
        assert!(details.storage_pvcs.temporary.is_none());
        assert_eq!(details.scheduling_events.len(), 1);
        assert_eq!(
            details.scheduling_events[0].details.scheduling_blockers,
            vec![crate::runtime_incidents::SchedulingBlocker::CpuCapacity]
        );
        assert_eq!(paths.lock().unwrap().len(), 3);
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
            StorageIdentity::from_claim(&claim)
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
        let expected = StorageIdentity::from_claim(&claim);
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
}
