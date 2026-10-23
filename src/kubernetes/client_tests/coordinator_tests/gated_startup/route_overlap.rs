use super::*;

fn route_path(coordinator: &KubernetesCoordinator, workspace: &Workspace) -> String {
    let name = coordinator
        .builder
        .runtime_names(workspace)
        .unwrap()
        .resources
        .web_shell_ingress;
    format!(
        "/apis/networking.k8s.io/v1/namespaces/{}/ingresses/{name}",
        workspace.runtime.namespace()
    )
}

async fn wait_for(signal: &Notify) {
    tokio::time::timeout(Duration::from_secs(3), signal.notified())
        .await
        .expect("the selected route or input operation must progress without releasing the other branch");
}

#[tokio::test]
async fn new_workspace_routes_overlap_inputs_and_approval_waits_for_both() {
    for routes_finish_first in [true, false] {
        let mut workspace = workspace(workspace_id());
        workspace.state = WorkspaceState::Provisioning;
        let mock = Arc::new(GateMock::default());
        mock.hold_files.store(true, Ordering::SeqCst);
        let coordinator = Arc::new(coordinator(mock.clone()));
        let route = route_path(&coordinator, &workspace);
        *mock.hold_request.lock().unwrap() = Some((Method::PATCH, route.clone()));
        let task = tokio::spawn({
            let (coordinator, workspace) = (coordinator.clone(), workspace.clone());
            async move { coordinator.reconcile(&workspace).await }
        });
        wait_for(&mock.files_pending).await;
        wait_for(&mock.request_started).await;
        wait_for_workload(&mock).await;
        assert_gated(&mock.object("/statefulsets/w-8000000000000001").unwrap());
        assert!(mock.object("-files-config").is_none());
        assert!(mock.object("-web-shell").is_none());
        assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 0);
        if routes_finish_first {
            mock.release_request.add_permits(1);
            wait_for(&mock.routes_applied).await;
            assert!(mock.object("-files-config").is_none());
        } else {
            mock.release_files.add_permits(1);
            wait_for(&mock.files_applied).await;
            assert!(mock.object("-web-shell").is_none());
        }
        assert!(mock.object("-startup-gate").is_none());
        assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 0);
        if routes_finish_first {
            mock.release_files.add_permits(1);
        } else {
            mock.release_request.add_permits(1);
        }
        tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 1);
        let writes = mock.writes.lock().unwrap();
        let approval = writes
            .iter()
            .position(|(path, _)| path.ends_with("-startup-gate"))
            .unwrap();
        for path in startup_prerequisite_paths(&coordinator.builder, &workspace)
            .into_iter()
            .chain([route])
        {
            assert!(
                writes
                    .iter()
                    .position(|(written, _)| written == &path)
                    .unwrap()
                    < approval
            );
        }
    }
}

#[tokio::test]
async fn failed_concurrent_routes_cancel_pending_inputs_without_approval() {
    let mut workspace = workspace(workspace_id());
    workspace.state = WorkspaceState::Provisioning;
    let mock = Arc::new(GateMock::default());
    mock.hold_files.store(true, Ordering::SeqCst);
    let coordinator = Arc::new(coordinator(mock.clone()));
    let route = route_path(&coordinator, &workspace);
    *mock.hold_request.lock().unwrap() = Some((Method::PATCH, route));
    *mock.fail_suffix.lock().unwrap() = Some("-web-shell".to_owned());
    let task = tokio::spawn({
        let (coordinator, workspace) = (coordinator.clone(), workspace.clone());
        async move { coordinator.reconcile(&workspace).await }
    });
    wait_for(&mock.files_pending).await;
    wait_for(&mock.request_started).await;
    wait_for_workload(&mock).await;
    let before = mock.object("/statefulsets/w-8000000000000001").unwrap();
    assert_gated(&before);
    mock.release_request.add_permits(1);
    let result = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .expect("route failure must cancel the blocked input branch")
        .unwrap();
    assert!(matches!(result, Err(ReconcileError::Kubernetes(_))));
    assert!(mock.object("-files-config").is_none());
    assert!(mock.object("-startup-gate").is_none());
    assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 0);
    mock.hold_files.store(false, Ordering::SeqCst);
    *mock.hold_request.lock().unwrap() = None;
    *mock.fail_suffix.lock().unwrap() = None;
    coordinator.reconcile(&workspace).await.unwrap();
    assert!(mock.object("-startup-gate").is_some());
    assert_eq!(
        before["spec"]["template"],
        mock.object("/statefulsets/w-8000000000000001").unwrap()["spec"]["template"]
    );
}

#[tokio::test]
async fn existing_workspaces_apply_inputs_before_starting_routes() {
    for approved in [false, true] {
        let mut workspace = workspace(workspace_id());
        workspace.state = WorkspaceState::Provisioning;
        let mock = Arc::new(GateMock::default());
        let coordinator = Arc::new(coordinator(mock.clone()));
        coordinator.reconcile(&workspace).await.unwrap();
        let route = route_path(&coordinator, &workspace);
        let names = coordinator
            .builder
            .runtime_names(&workspace)
            .unwrap()
            .resources;
        let files = format!(
            "/api/v1/namespaces/{}/configmaps/{}",
            workspace.runtime.namespace(),
            names.files_config_map
        );
        {
            let mut objects = mock.objects.lock().unwrap();
            objects.remove(&route).unwrap();
            objects.remove(&files).unwrap();
            if !approved {
                let gate = format!(
                    "/api/v1/namespaces/{}/configmaps/{}",
                    workspace.runtime.namespace(),
                    names.startup_gate_config_map()
                );
                objects.remove(&gate).unwrap();
            }
        }
        if approved {
            workspace.state = WorkspaceState::Ready;
        }
        mock.events.lock().unwrap().clear();
        mock.hold_files.store(true, Ordering::SeqCst);
        let task = tokio::spawn({
            let (coordinator, workspace) = (coordinator.clone(), workspace.clone());
            async move { coordinator.reconcile(&workspace).await }
        });
        wait_for(&mock.files_pending).await;
        assert!(
            !mock
                .events
                .lock()
                .unwrap()
                .iter()
                .any(|(_, path)| path == &route)
        );
        mock.release_files.add_permits(1);
        tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let events = mock.events.lock().unwrap();
        let applied = events
            .iter()
            .position(|(operation, path)| operation == "APPLIED" && path == &files)
            .unwrap();
        let read_route = events
            .iter()
            .position(|(operation, path)| operation == "GET" && path == &route)
            .unwrap();
        assert!(
            applied < read_route,
            "existing inputs must finish before the first route request"
        );
        assert_eq!(
            mock.approval_creates.load(Ordering::SeqCst),
            if approved { 1 } else { 2 }
        );
    }
}
