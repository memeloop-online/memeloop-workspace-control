use super::*;

fn provisioning_workspace() -> Workspace {
    let mut workspace = workspace(workspace_id());
    workspace.state = WorkspaceState::Provisioning;
    workspace.template.cluster_access = true;
    workspace
}

fn prerequisite_paths(coordinator: &KubernetesCoordinator, workspace: &Workspace) -> Vec<String> {
    let names = coordinator
        .builder
        .runtime_names(workspace)
        .unwrap()
        .resources;
    let namespace = workspace.runtime.namespace();
    [
        ("secrets", names.ssh_identity_secret),
        ("configmaps", names.workspace_config),
        ("services", names.service),
    ]
    .into_iter()
    .map(|(kind, name)| format!("/api/v1/namespaces/{namespace}/{kind}/{name}"))
    .collect()
}

async fn wait_for_request(mock: &GateMock) {
    tokio::time::timeout(Duration::from_secs(3), mock.request_started.notified())
        .await
        .expect("the selected prerequisite request must be pending");
}

fn workload_patch_count(mock: &GateMock) -> usize {
    mock.patches
        .lock()
        .unwrap()
        .iter()
        .filter(|(path, _)| path.contains("/statefulsets/"))
        .count()
}

#[tokio::test]
async fn volumes_can_prepare_while_identity_or_service_writes_are_pending() {
    for index in 0..3 {
        let workspace = provisioning_workspace();
        let mock = Arc::new(GateMock::default());
        let coordinator = Arc::new(coordinator(mock.clone()));
        let held = prerequisite_paths(&coordinator, &workspace)[index].clone();
        *mock.hold_request.lock().unwrap() = Some((Method::PATCH, held.clone()));
        let task = tokio::spawn({
            let coordinator = coordinator.clone();
            let workspace = workspace.clone();
            async move { coordinator.reconcile(&workspace).await }
        });
        wait_for_request(&mock).await;
        wait_for_workload(&mock).await;
        assert_gated(&mock.object("/statefulsets/w-8000000000000001").unwrap());
        assert!(!mock.objects.lock().unwrap().contains_key(&held));
        assert!(mock.object("-startup-gate").is_none());
        assert!(mock.object("-web-shell").is_none());
        // Admission identity and network isolation were established before the Pod template.
        let names = coordinator
            .builder
            .runtime_names(&workspace)
            .unwrap()
            .resources;
        assert!(
            mock.object(&format!("/serviceaccounts/{}", names.service_account))
                .is_some()
        );
        assert!(
            mock.object(&format!("/networkpolicies/{}", names.network_policy))
                .is_some()
        );
        mock.release_request.add_permits(1);
        task.await.unwrap().unwrap();
        assert!(mock.object("-startup-gate").is_some());
        let patches = mock.writes.lock().unwrap();
        let input_position = patches.iter().position(|(path, _)| path == &held).unwrap();
        let gate_position = patches
            .iter()
            .position(|(path, _)| path.ends_with("-startup-gate"))
            .unwrap();
        assert!(input_position < gate_position);
    }
}

#[tokio::test]
async fn access_and_isolation_must_finish_before_early_workload_creation() {
    for kind in ["serviceaccounts", "networkpolicies"] {
        let workspace = provisioning_workspace();
        let mock = Arc::new(GateMock::default());
        let coordinator = Arc::new(coordinator(mock.clone()));
        let names = coordinator
            .builder
            .runtime_names(&workspace)
            .unwrap()
            .resources;
        let name = if kind == "serviceaccounts" {
            names.service_account
        } else {
            names.network_policy
        };
        let prefix = if kind == "serviceaccounts" {
            "/api/v1"
        } else {
            "/apis/networking.k8s.io/v1"
        };
        let held = format!(
            "{prefix}/namespaces/{}/{kind}/{name}",
            workspace.runtime.namespace()
        );
        *mock.hold_request.lock().unwrap() = Some((Method::PATCH, held));
        let task = tokio::spawn({
            let coordinator = coordinator.clone();
            let workspace = workspace.clone();
            async move { coordinator.reconcile(&workspace).await }
        });
        wait_for_request(&mock).await;
        assert_eq!(workload_patch_count(&mock), 0);
        assert!(mock.object("-startup-gate").is_none());
        mock.release_request.add_permits(1);
        task.await.unwrap().unwrap();
        assert_eq!(workload_patch_count(&mock), 1);
    }
}

#[tokio::test]
async fn a_late_input_failure_or_foreign_owner_never_approves_the_early_workload() {
    for (index, foreign) in
        (0..3).flat_map(|index| [false, true].map(move |foreign| (index, foreign)))
    {
        let workspace = provisioning_workspace();
        let mock = Arc::new(GateMock::default());
        let coordinator = Arc::new(coordinator(mock.clone()));
        let held = prerequisite_paths(&coordinator, &workspace)[index].clone();
        let method = if foreign { Method::GET } else { Method::PATCH };
        *mock.hold_request.lock().unwrap() = Some((method, held.clone()));
        let foreign_object = json!({"apiVersion":"v1", "kind":"PartialObjectMetadata", "metadata":{
            "labels":ownership_labels("another-installation", workspace.id)
        }});
        if foreign {
            mock.objects
                .lock()
                .unwrap()
                .insert(held.clone(), foreign_object.clone());
        } else {
            *mock.fail_suffix.lock().unwrap() = Some(held.clone());
        }
        let task = tokio::spawn({
            let coordinator = coordinator.clone();
            let workspace = workspace.clone();
            async move { coordinator.reconcile(&workspace).await }
        });
        wait_for_request(&mock).await;
        wait_for_workload(&mock).await;
        let before = mock.object("/statefulsets/w-8000000000000001").unwrap();
        assert_gated(&before);
        mock.release_request.add_permits(1);
        let error = task.await.unwrap().unwrap_err();
        if foreign {
            assert!(matches!(error, ReconcileError::Ownership(_)));
            assert_eq!(
                mock.objects.lock().unwrap().get(&held),
                Some(&foreign_object)
            );
        } else {
            assert!(matches!(error, ReconcileError::Kubernetes(_)));
        }
        assert!(mock.object("-startup-gate").is_none());
        assert!(mock.object("-web-shell").is_none());
        assert_eq!(
            before,
            mock.object("/statefulsets/w-8000000000000001").unwrap()
        );
        if !foreign {
            *mock.hold_request.lock().unwrap() = None;
            *mock.fail_suffix.lock().unwrap() = None;
            coordinator.reconcile(&workspace).await.unwrap();
            assert!(mock.object("-startup-gate").is_some());
            assert_eq!(
                before["spec"]["template"],
                mock.object("/statefulsets/w-8000000000000001").unwrap()["spec"]["template"]
            );
        }
    }
}

#[tokio::test]
async fn previously_approved_workloads_wait_for_pending_inputs_before_being_applied_again() {
    let workspace = provisioning_workspace();
    let mock = Arc::new(GateMock::default());
    let coordinator = Arc::new(coordinator(mock.clone()));
    coordinator.reconcile(&workspace).await.unwrap();
    assert_eq!(workload_patch_count(&mock), 1);
    let before = mock.object("/statefulsets/w-8000000000000001").unwrap();
    let held = prerequisite_paths(&coordinator, &workspace)[0].clone();
    *mock.hold_request.lock().unwrap() = Some((Method::PATCH, held));
    let task = tokio::spawn({
        let coordinator = coordinator.clone();
        let workspace = workspace.clone();
        async move { coordinator.reconcile(&workspace).await }
    });
    wait_for_request(&mock).await;
    assert_eq!(workload_patch_count(&mock), 1);
    mock.release_request.add_permits(1);
    task.await.unwrap().unwrap();
    assert_eq!(workload_patch_count(&mock), 2);
    assert_eq!(
        before["spec"]["template"],
        mock.object("/statefulsets/w-8000000000000001").unwrap()["spec"]["template"]
    );
}

#[tokio::test]
async fn a_foreign_workload_is_not_adopted_for_early_volume_preparation() {
    let workspace = provisioning_workspace();
    let mock = Arc::new(GateMock::default());
    let coordinator = coordinator(mock.clone());
    let mut workload = coordinator.builder.build(&workspace).unwrap().stateful_set;
    workload.metadata.labels = Some(
        serde_json::from_value(ownership_labels("another-installation", workspace.id)).unwrap(),
    );
    let path = format!(
        "/apis/apps/v1/namespaces/{}/statefulsets/{}",
        workspace.runtime.namespace(),
        workload.metadata.name.as_ref().unwrap()
    );
    let foreign = serde_json::to_value(workload).unwrap();
    mock.objects.lock().unwrap().insert(path, foreign.clone());
    assert!(matches!(
        coordinator.reconcile(&workspace).await,
        Err(ReconcileError::Ownership(_))
    ));
    assert_eq!(workload_patch_count(&mock), 0);
    assert_eq!(
        foreign,
        mock.object("/statefulsets/w-8000000000000001").unwrap()
    );
    assert!(mock.object("-startup-gate").is_none());
}
