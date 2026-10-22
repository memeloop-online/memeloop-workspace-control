use super::*;

fn approval_path(coordinator: &KubernetesCoordinator, workspace: &Workspace) -> String {
    let name = coordinator
        .builder
        .runtime_names(workspace)
        .unwrap()
        .resources
        .startup_gate_config_map();
    format!(
        "/api/v1/namespaces/{}/configmaps/{name}",
        workspace.runtime.namespace()
    )
}

fn assert_no_approval_patch(mock: &GateMock) {
    assert!(
        !mock
            .patches
            .lock()
            .unwrap()
            .iter()
            .any(|(path, _)| path.ends_with("-startup-gate"))
    );
}

#[tokio::test]
async fn approval_created_between_read_and_post_is_not_overwritten_and_retry_revalidates_it() {
    for (foreign, approved, immutable) in [
        (true, "true", true),
        (false, "false", true),
        (false, "true", false),
        (false, "true", true),
    ] {
        let mut workspace = workspace(workspace_id());
        workspace.state = WorkspaceState::Provisioning;
        let mock = Arc::new(GateMock::default());
        mock.hold_files.store(true, Ordering::SeqCst);
        let coordinator = Arc::new(coordinator(mock.clone()));
        let path = approval_path(&coordinator, &workspace);
        let task = tokio::spawn({
            let (coordinator, workspace) = (coordinator.clone(), workspace.clone());
            async move { coordinator.reconcile(&workspace).await }
        });
        wait_for_workload(&mock).await;
        assert!(mock.object("-startup-gate").is_none());
        assert_eq!(mock.approval_reads.load(Ordering::SeqCst), 1);
        assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 0);
        let workload = mock.object("/statefulsets/w-8000000000000001").unwrap();
        assert_gated(&workload);
        let gate = json!({"apiVersion":"v1", "kind":"ConfigMap", "metadata":{
            "name":path.rsplit('/').next().unwrap(), "uid":"concurrent-gate",
            "labels":ownership_labels(if foreign {"another-installation"} else {"public-a"}, workspace.id)
        }, "immutable":immutable, "data":{"approved":approved}});
        mock.objects
            .lock()
            .unwrap()
            .insert(path.clone(), gate.clone());
        mock.hold_files.store(false, Ordering::SeqCst);
        mock.release_files.add_permits(1);
        assert!(
            matches!(task.await.unwrap(), Err(ReconcileError::Kubernetes(kube::Error::Api(error))) if error.code == 409)
        );
        assert_eq!(
            mock.approval_creates.load(Ordering::SeqCst),
            1,
            "a conflict must not trigger an inline retry"
        );
        assert_eq!(mock.objects.lock().unwrap().get(&path), Some(&gate));
        assert_no_approval_patch(&mock);

        let result = coordinator.reconcile(&workspace).await;
        if foreign {
            assert!(matches!(result, Err(ReconcileError::Ownership(_))));
        } else if approved != "true" || !immutable {
            assert!(matches!(
                result,
                Err(ReconcileError::RuntimeIdentityMismatch)
            ));
        } else {
            result.unwrap();
        }
        assert_eq!(mock.approval_reads.load(Ordering::SeqCst), 2);
        assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 1);
        assert_eq!(mock.objects.lock().unwrap().get(&path), Some(&gate));
        assert_eq!(
            workload["spec"]["template"],
            mock.object("/statefulsets/w-8000000000000001").unwrap()["spec"]["template"]
        );
        assert_no_approval_patch(&mock);
    }
}

#[tokio::test]
async fn persisted_approval_with_lost_response_is_verified_on_next_reconcile_without_another_post()
{
    let mut workspace = workspace(workspace_id());
    workspace.state = WorkspaceState::Provisioning;
    let mock = Arc::new(GateMock::default());
    mock.lose_approval_response.store(true, Ordering::SeqCst);
    let coordinator = coordinator(mock.clone());
    // The mock commits create first, then the response body fails with TimedOut.
    // This must remain an error even though the next reconciliation can recover.
    assert!(matches!(coordinator.reconcile(&workspace).await,
        Err(ReconcileError::Kubernetes(error)) if !matches!(error, kube::Error::Api(_))));
    let gate = mock
        .object("-startup-gate")
        .expect("server committed before response loss");
    let workload = mock.object("/statefulsets/w-8000000000000001").unwrap();
    assert_gated(&workload);
    assert_eq!(gate["immutable"], true);
    assert_eq!(gate["data"]["approved"], "true");
    assert_eq!(mock.approval_reads.load(Ordering::SeqCst), 1);
    assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 1);
    coordinator.reconcile(&workspace).await.unwrap();
    assert_eq!(mock.approval_reads.load(Ordering::SeqCst), 2);
    assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 1);
    assert_eq!(mock.object("-startup-gate").unwrap(), gate);
    assert_eq!(
        workload["spec"]["template"],
        mock.object("/statefulsets/w-8000000000000001").unwrap()["spec"]["template"]
    );
    assert_no_approval_patch(&mock);
}

#[tokio::test]
async fn deleted_approval_is_recreated_only_after_prerequisites_finish() {
    let mut workspace = workspace(workspace_id());
    workspace.state = WorkspaceState::Provisioning;
    let mock = Arc::new(GateMock::default());
    let coordinator = Arc::new(coordinator(mock.clone()));
    coordinator.reconcile(&workspace).await.unwrap();
    let path = approval_path(&coordinator, &workspace);
    let old = mock.objects.lock().unwrap().remove(&path).unwrap();
    let workload = mock.object("/statefulsets/w-8000000000000001").unwrap();
    // Discard the first reconciliation's notification before waiting for reapply.
    wait_for_workload(&mock).await;
    mock.hold_files.store(true, Ordering::SeqCst);
    let task = tokio::spawn({
        let (coordinator, workspace) = (coordinator.clone(), workspace.clone());
        async move { coordinator.reconcile(&workspace).await }
    });
    wait_for_workload(&mock).await;
    assert!(mock.object("-startup-gate").is_none());
    assert_eq!(mock.approval_reads.load(Ordering::SeqCst), 2);
    assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 1);
    mock.hold_files.store(false, Ordering::SeqCst);
    mock.release_files.add_permits(1);
    task.await.unwrap().unwrap();
    let recreated = mock.object("-startup-gate").unwrap();
    assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 2);
    assert_ne!(old["metadata"]["uid"], recreated["metadata"]["uid"]);
    assert_eq!(old["immutable"], recreated["immutable"]);
    assert_eq!(old["data"], recreated["data"]);
    assert_eq!(old["metadata"]["labels"], recreated["metadata"]["labels"]);
    assert_eq!(
        workload["spec"]["template"],
        mock.object("/statefulsets/w-8000000000000001").unwrap()["spec"]["template"]
    );
    assert_no_approval_patch(&mock);
}

#[tokio::test]
async fn terminating_approval_collision_blocks_create_until_the_object_is_gone() {
    let mut workspace = workspace(workspace_id());
    workspace.state = WorkspaceState::Provisioning;
    let mock = Arc::new(GateMock::default());
    mock.hold_files.store(true, Ordering::SeqCst);
    let coordinator = Arc::new(coordinator(mock.clone()));
    let path = approval_path(&coordinator, &workspace);
    let task = tokio::spawn({
        let (coordinator, workspace) = (coordinator.clone(), workspace.clone());
        async move { coordinator.reconcile(&workspace).await }
    });
    wait_for_workload(&mock).await;
    let gate = json!({"apiVersion":"v1", "kind":"ConfigMap", "metadata":{
        "name":path.rsplit('/').next().unwrap(), "uid":"terminating-gate",
        "labels":ownership_labels("public-a", workspace.id),
        "deletionTimestamp":"2026-10-22T00:00:00Z", "finalizers":["test.example/hold"]
    }, "immutable":true, "data":{"approved":"true"}});
    mock.objects
        .lock()
        .unwrap()
        .insert(path.clone(), gate.clone());
    mock.hold_files.store(false, Ordering::SeqCst);
    mock.release_files.add_permits(1);
    assert!(
        matches!(task.await.unwrap(), Err(ReconcileError::Kubernetes(kube::Error::Api(error))) if error.code == 409)
    );
    assert_eq!(mock.objects.lock().unwrap().get(&path), Some(&gate));
    assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 1);
    assert_no_approval_patch(&mock);
    // Kubernetes finishes deletion; the next full GET now sees real absence.
    assert_eq!(mock.objects.lock().unwrap().remove(&path), Some(gate));
    coordinator.reconcile(&workspace).await.unwrap();
    assert_eq!(mock.approval_reads.load(Ordering::SeqCst), 2);
    assert_eq!(mock.approval_creates.load(Ordering::SeqCst), 2);
    assert_ne!(
        mock.object("-startup-gate").unwrap()["metadata"]["uid"],
        "terminating-gate"
    );
    assert_no_approval_patch(&mock);
}
