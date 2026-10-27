use super::*;

async fn wait_for_plan_reads(mock: &GateMock) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while mock.approval_reads.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("read-only startup planning must progress while the namespace is pending");
    assert_eq!(mock.workload_reads.load(Ordering::SeqCst), 1);
    assert_eq!(mock.approval_reads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn pending_namespace_allows_only_plan_reads_until_creation_succeeds() {
    let mut workspace = workspace(workspace_id());
    workspace.state = WorkspaceState::Provisioning;
    let mock = Arc::new(GateMock::default());
    let coordinator = Arc::new(coordinator(mock.clone()));
    let namespace = format!("/api/v1/namespaces/{}", workspace.runtime.namespace());
    *mock.hold_request.lock().unwrap() = Some((Method::GET, namespace.clone()));
    let task = tokio::spawn({
        let (coordinator, workspace) = (coordinator.clone(), workspace.clone());
        async move { coordinator.reconcile(&workspace).await }
    });
    wait_for_plan_reads(&mock).await;
    assert!(mock.writes.lock().unwrap().is_empty());
    assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 0);
    {
        let events = mock.events.lock().unwrap();
        assert_eq!(events.len(), 3);
        assert!(events.iter().all(|(method, _)| method == "GET"));
        assert!(events.iter().any(|(_, path)| path == &namespace));
        assert!(
            events
                .iter()
                .any(|(_, path)| path.contains("/statefulsets/"))
        );
        assert!(
            events
                .iter()
                .any(|(_, path)| path.ends_with("-startup-gate"))
        );
    }
    mock.release_request.add_permits(1);
    task.await.unwrap().unwrap();
    let writes = mock.writes.lock().unwrap();
    assert_eq!(writes.first().unwrap().0, namespace);
    assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 1);
    assert_gated(&mock.object("/statefulsets/w-8000000000000001").unwrap());
    assert_eq!(
        mock.object("-startup-gate").unwrap()["data"]["approved"],
        "true"
    );
}

#[tokio::test]
async fn failed_or_foreign_namespace_never_writes_workspace_resources_after_plan_reads() {
    for failure in ["get", "patch", "foreign"] {
        let mut workspace = workspace(workspace_id());
        workspace.state = WorkspaceState::Provisioning;
        let mock = Arc::new(GateMock::default());
        let coordinator = Arc::new(coordinator(mock.clone()));
        let namespace = format!("/api/v1/namespaces/{}", workspace.runtime.namespace());
        let method = if failure == "patch" {
            Method::PATCH
        } else {
            Method::GET
        };
        *mock.hold_request.lock().unwrap() = Some((method.clone(), namespace.clone()));
        if failure == "foreign" {
            let mut foreign =
                serde_json::to_value(coordinator.builder.build(&workspace).unwrap().namespace)
                    .unwrap();
            foreign["metadata"]["labels"]["workspace.memeloop.dev/owner-installation"] =
                json!("another-installation");
            mock.objects
                .lock()
                .unwrap()
                .insert(namespace.clone(), foreign);
        } else {
            *mock.fail_request.lock().unwrap() = Some((method, namespace));
        }
        let before = mock.objects.lock().unwrap().clone();
        let task = tokio::spawn({
            let (coordinator, workspace) = (coordinator.clone(), workspace.clone());
            async move { coordinator.reconcile(&workspace).await }
        });
        wait_for_plan_reads(&mock).await;
        assert!(mock.writes.lock().unwrap().is_empty());
        mock.release_request.add_permits(1);
        let error = task.await.unwrap().unwrap_err();
        if failure == "foreign" {
            assert!(matches!(error, ReconcileError::Ownership(_)));
        } else {
            assert!(matches!(error, ReconcileError::Kubernetes(_)));
        }
        assert!(mock.writes.lock().unwrap().is_empty());
        assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 0);
        assert_eq!(*mock.objects.lock().unwrap(), before);
    }
}
