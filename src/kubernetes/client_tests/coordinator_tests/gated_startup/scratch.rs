use super::*;

#[tokio::test]
async fn pending_scratch_binds_after_consumer_and_is_reused_without_writes() {
    let mut workspace = workspace(workspace_id());
    workspace.state = WorkspaceState::Provisioning;
    let mock = Arc::new(GateMock::default());
    let coordinator = coordinator(mock.clone());
    coordinator.reconcile(&workspace).await.unwrap();
    let claim = mock.object("-scratch").unwrap();
    assert!(
        claim.get("status").is_none(),
        "WFFC need not be Bound before publishing its consumer"
    );
    assert!(claim["metadata"].get("ownerReferences").is_none());
    let workload = mock.object("/statefulsets/w-8000000000000001").unwrap();
    let scratch = workload["spec"]["template"]["spec"]["volumes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "workspace-scratch")
        .unwrap();
    assert_eq!(
        scratch["persistentVolumeClaim"]["claimName"],
        claim["metadata"]["name"]
    );
    assert!(scratch.get("ephemeral").is_none());
    mock.events.lock().unwrap().clear();
    coordinator.reconcile(&workspace).await.unwrap();
    assert_eq!(mock.object("-scratch"), Some(claim));
    assert!(
        mock.events
            .lock()
            .unwrap()
            .iter()
            .all(|(method, path)| !path.contains("/persistentvolumeclaims") || method == "GET")
    );
}

#[tokio::test]
async fn scratch_creation_can_progress_with_workload_without_waiting_for_bound() {
    let mut workspace = workspace(workspace_id());
    workspace.state = WorkspaceState::Provisioning;
    let mock = Arc::new(GateMock::default());
    let coordinator = Arc::new(coordinator(mock.clone()));
    *mock.hold_request.lock().unwrap() = Some((
        Method::POST,
        format!(
            "/api/v1/namespaces/{}/persistentvolumeclaims",
            workspace.runtime.namespace()
        ),
    ));
    let task = tokio::spawn({
        let coordinator = coordinator.clone();
        let workspace = workspace.clone();
        async move { coordinator.reconcile(&workspace).await }
    });
    tokio::time::timeout(Duration::from_secs(3), mock.request_started.notified())
        .await
        .unwrap();
    wait_for_workload(&mock).await;
    assert!(mock.object("-scratch").is_none());
    assert!(
        mock.object("-startup-gate").is_none(),
        "claim materialization must complete before approval"
    );
    mock.release_request.add_permits(1);
    task.await.unwrap().unwrap();
    assert!(mock.object("-scratch").is_some());
}

#[tokio::test]
async fn foreign_or_terminating_scratch_is_not_recreated_or_overwritten() {
    for foreign in [false, true] {
        let mut workspace = workspace(workspace_id());
        workspace.state = WorkspaceState::Provisioning;
        let mock = Arc::new(GateMock::default());
        let coordinator = coordinator(mock.clone());
        let claim = coordinator
            .builder
            .build(&workspace)
            .unwrap()
            .scratch_claim
            .unwrap();
        let name = claim.metadata.name.clone().unwrap();
        let path = format!(
            "/api/v1/namespaces/{}/persistentvolumeclaims/{name}",
            workspace.runtime.namespace()
        );
        let mut claim = serde_json::to_value(claim).unwrap();
        claim["metadata"]["uid"] = json!("original-scratch-uid");
        if foreign {
            claim["metadata"]["labels"]["workspace.memeloop.dev/owner-installation"] =
                json!("foreign");
        } else {
            claim["metadata"]["deletionTimestamp"] = json!("2026-10-28T00:00:00Z");
        }
        mock.objects.lock().unwrap().insert(path, claim.clone());
        let result = coordinator.reconcile(&workspace).await;
        if foreign {
            assert!(matches!(result, Err(ReconcileError::Ownership(_))));
        } else {
            assert!(matches!(
                result,
                Err(ReconcileError::ScratchVolumeRecreationPending)
            ));
        }
        assert_eq!(mock.object("-scratch"), Some(claim));
        assert!(mock.object("-startup-gate").is_none());
        assert!(
            mock.events
                .lock()
                .unwrap()
                .iter()
                .all(|(method, path)| !path.contains("/persistentvolumeclaims") || method == "GET")
        );
    }
}
