use super::*;

fn tokenless_workspace() -> Workspace {
    let mut workspace = workspace(workspace_id());
    workspace.state = WorkspaceState::Provisioning;
    workspace.template.cluster_access = false;
    workspace
}

fn identity_paths(coordinator: &KubernetesCoordinator, workspace: &Workspace) -> [String; 2] {
    let names = coordinator
        .builder
        .runtime_names(workspace)
        .unwrap()
        .resources;
    [
        format!(
            "/apis/rbac.authorization.k8s.io/v1/clusterrolebindings/{}",
            coordinator
                .builder
                .cluster_admin_binding_name(workspace)
                .unwrap()
        ),
        format!(
            "/api/v1/namespaces/{}/serviceaccounts/{}",
            workspace.runtime.namespace(),
            names.service_account
        ),
    ]
}

fn seed_old_identity(mock: &GateMock, coordinator: &KubernetesCoordinator, workspace: &Workspace) {
    let mut previous = workspace.clone();
    previous.template.cluster_access = true;
    let desired = coordinator.builder.build(&previous).unwrap();
    let [binding, account] = identity_paths(coordinator, workspace);
    let mut objects = mock.objects.lock().unwrap();
    objects.insert(
        binding,
        serde_json::to_value(desired.cluster_role_binding.unwrap()).unwrap(),
    );
    objects.insert(
        account,
        serde_json::to_value(desired.service_account.unwrap()).unwrap(),
    );
}

async fn wait_for_identity_request(mock: &GateMock) {
    tokio::time::timeout(Duration::from_secs(3), mock.request_started.notified())
        .await
        .expect("the selected identity or isolation request must be pending");
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
async fn new_tokenless_workload_prepares_volumes_while_old_identity_cleanup_is_pending() {
    for index in 0..2 {
        let workspace = tokenless_workspace();
        let mock = Arc::new(GateMock::default());
        let coordinator = Arc::new(coordinator(mock.clone()));
        seed_old_identity(&mock, &coordinator, &workspace);
        let held = identity_paths(&coordinator, &workspace)[index].clone();
        *mock.hold_request.lock().unwrap() = Some((Method::GET, held.clone()));
        let task = tokio::spawn({
            let (coordinator, workspace) = (coordinator.clone(), workspace.clone());
            async move { coordinator.reconcile(&workspace).await }
        });
        wait_for_identity_request(&mock).await;
        wait_for_workload(&mock).await;
        let workload = mock.object("/statefulsets/w-8000000000000001").unwrap();
        assert_gated(&workload);
        let pod = &workload["spec"]["template"]["spec"];
        assert!(pod["serviceAccountName"].is_null());
        assert_eq!(pod["automountServiceAccountToken"], false);
        assert!(mock.objects.lock().unwrap().contains_key(&held));
        assert!(mock.object("-startup-gate").is_none());
        assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 0);
        mock.release_request.add_permits(1);
        task.await.unwrap().unwrap();
        assert!(!mock.objects.lock().unwrap().contains_key(&held));
        assert_eq!(workload_patch_count(&mock), 1);
        assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 1);
        let events = mock.events.lock().unwrap();
        let cleanup = events
            .iter()
            .position(|(method, path)| method == "DELETE" && path == &held)
            .unwrap();
        let approval = events
            .iter()
            .position(|(method, _)| method == "POST")
            .unwrap();
        assert!(cleanup < approval);
    }
}

#[tokio::test]
async fn cleanup_errors_or_foreign_identity_never_approve_and_retry_preserves_the_template() {
    for failure in ["get", "delete", "foreign"] {
        let workspace = tokenless_workspace();
        let mock = Arc::new(GateMock::default());
        let coordinator = Arc::new(coordinator(mock.clone()));
        seed_old_identity(&mock, &coordinator, &workspace);
        let held = identity_paths(&coordinator, &workspace)[0].clone();
        let method = if failure == "delete" {
            Method::DELETE
        } else {
            Method::GET
        };
        *mock.hold_request.lock().unwrap() = Some((method.clone(), held.clone()));
        if failure == "foreign" {
            mock.objects.lock().unwrap().get_mut(&held).unwrap()["metadata"]["labels"] =
                ownership_labels("another-installation", workspace.id);
        } else {
            *mock.fail_request.lock().unwrap() = Some((method, held.clone()));
        }
        let identity = mock.objects.lock().unwrap().get(&held).unwrap().clone();
        let task = tokio::spawn({
            let (coordinator, workspace) = (coordinator.clone(), workspace.clone());
            async move { coordinator.reconcile(&workspace).await }
        });
        wait_for_identity_request(&mock).await;
        wait_for_workload(&mock).await;
        let before = mock.object("/statefulsets/w-8000000000000001").unwrap();
        assert_gated(&before);
        mock.release_request.add_permits(1);
        let error = task.await.unwrap().unwrap_err();
        if failure == "foreign" {
            assert!(matches!(error, ReconcileError::Ownership(_)));
        } else {
            assert!(matches!(error, ReconcileError::Kubernetes(_)));
        }
        assert_eq!(mock.objects.lock().unwrap().get(&held), Some(&identity));
        assert!(mock.object("-startup-gate").is_none());
        assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 0);
        *mock.hold_request.lock().unwrap() = None;
        *mock.fail_request.lock().unwrap() = None;
        if failure == "foreign" {
            mock.objects.lock().unwrap().remove(&held);
        }
        coordinator.reconcile(&workspace).await.unwrap();
        assert!(mock.object("-startup-gate").is_some());
        assert_eq!(
            before["spec"]["template"],
            mock.object("/statefulsets/w-8000000000000001").unwrap()["spec"]["template"]
        );
    }
}

#[tokio::test]
async fn tokenless_workload_still_waits_for_network_isolation_before_early_creation() {
    let workspace = tokenless_workspace();
    let mock = Arc::new(GateMock::default());
    let coordinator = Arc::new(coordinator(mock.clone()));
    let names = coordinator
        .builder
        .runtime_names(&workspace)
        .unwrap()
        .resources;
    let policy = format!(
        "/apis/networking.k8s.io/v1/namespaces/{}/networkpolicies/{}",
        workspace.runtime.namespace(),
        names.network_policy
    );
    *mock.hold_request.lock().unwrap() = Some((Method::PATCH, policy.clone()));
    let task = tokio::spawn({
        let (coordinator, workspace) = (coordinator.clone(), workspace.clone());
        async move { coordinator.reconcile(&workspace).await }
    });
    wait_for_identity_request(&mock).await;
    assert_eq!(workload_patch_count(&mock), 0);
    assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 0);
    mock.release_request.add_permits(1);
    task.await.unwrap().unwrap();
    let events = mock.events.lock().unwrap();
    let isolation = events
        .iter()
        .position(|(operation, path)| operation == "APPLIED" && path == &policy)
        .unwrap();
    let workload = events
        .iter()
        .position(|(operation, path)| operation == "APPLIED" && path.contains("/statefulsets/"))
        .unwrap();
    assert!(isolation < workload);
}

#[tokio::test]
async fn existing_and_approved_tokenless_workloads_keep_identity_before_workload() {
    for (existing, approved) in [(true, false), (true, true), (false, true)] {
        let workspace = tokenless_workspace();
        let mock = Arc::new(GateMock::default());
        let coordinator = Arc::new(coordinator(mock.clone()));
        coordinator.reconcile(&workspace).await.unwrap();
        let names = coordinator
            .builder
            .runtime_names(&workspace)
            .unwrap()
            .resources;
        if !existing {
            mock.objects
                .lock()
                .unwrap()
                .remove(&format!(
                    "/apis/apps/v1/namespaces/{}/statefulsets/{}",
                    workspace.runtime.namespace(),
                    names.stateful_set
                ))
                .unwrap();
        }
        if !approved {
            mock.objects
                .lock()
                .unwrap()
                .remove(&format!(
                    "/api/v1/namespaces/{}/configmaps/{}",
                    workspace.runtime.namespace(),
                    names.startup_gate_config_map()
                ))
                .unwrap();
        }
        let held = identity_paths(&coordinator, &workspace)[0].clone();
        *mock.hold_request.lock().unwrap() = Some((Method::GET, held));
        let task = tokio::spawn({
            let (coordinator, workspace) = (coordinator.clone(), workspace.clone());
            async move { coordinator.reconcile(&workspace).await }
        });
        wait_for_identity_request(&mock).await;
        assert_eq!(workload_patch_count(&mock), 1);
        assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 1);
        mock.release_request.add_permits(1);
        task.await.unwrap().unwrap();
        assert_eq!(workload_patch_count(&mock), 2);
        assert_eq!(
            mock.approval_creates.load(Ordering::SeqCst),
            if approved { 1 } else { 2 }
        );
    }
}
