use super::*;
use crate::{
    auth::ApiKeyScope,
    crypto::EnvelopeCipher,
    injections::{InjectionItem, InjectionKind, InjectionScope, InjectionValue},
    jobs::{JobHandler, JobHandlerError, WorkspaceReconcileHandler},
    storage::{
        ClaimedJob, CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate, Database,
        InjectionScopeRef,
    },
    templates::WorkspaceTemplateDocument,
    workspaces::{WorkspaceAction, WorkspaceObservation},
};

async fn seed(database: &Database) -> Workspace {
    database.migrate().await.unwrap();
    database
        .upsert_image_policy("example/workspace:1", true, 1)
        .await
        .unwrap();
    let owner = database
        .create_user_with_initial_key(
            "Injection updates",
            "injection-update-token-0000000000000000",
            true,
            ApiKeyScope::initial_key_defaults(true),
            3600,
            1,
        )
        .await
        .unwrap()
        .user_id;
    let org = database
        .create_organization(
            CreateOrganization {
                name: "Injection updates".into(),
                owner_user_id: owner,
            },
            1,
        )
        .await
        .unwrap();
    let template = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(org.id),
                yaml: WorkspaceTemplateDocument::new(
                    "Injection updates",
                    workspace(workspace_id()).template,
                )
                .to_yaml()
                .unwrap(),
            },
            true,
            1,
        )
        .await
        .unwrap();
    let workspace = database
        .create_workspace(
            CreateWorkspace {
                organization_id: org.id,
                owner_id: owner,
                name: "Injection updates".into(),
                template_id: template.id,
                resources: None,
                temporary_storage_gib: None,
                use_longhorn: false,
                organization_injection_refs: None,
                user_injection_refs: None,
            },
            true,
            owner,
            1,
        )
        .await
        .unwrap();
    database
        .record_workspace_observation(
            workspace.id,
            workspace.generation,
            WorkspaceObservation::Ready,
            owner,
            2,
        )
        .await
        .unwrap()
        .unwrap()
}

fn job(workspace: &Workspace) -> ClaimedJob {
    ClaimedJob {
        id: Uuid::now_v7(),
        kind: "reconcile_workspace".into(),
        workspace_id: Some(workspace.id),
        payload: json!({"reason":"injection_changed", "generation":workspace.generation}),
        attempts: 1,
        lease_expires_at: i64::MAX,
    }
}

fn seed_workload(mock: &GateMock, coordinator: &KubernetesCoordinator, workspace: &Workspace) {
    let desired = coordinator.builder.build(workspace).unwrap();
    let mut old = serde_json::to_value(desired.stateful_set).unwrap();
    // Represent a running workspace rendered before unrelated bootstrap/probe changes.
    let spec = &mut old["spec"]["template"]["spec"];
    spec["containers"][0]["env"] = json!([{"name":"RUSTUP_HOME","value":"/opt/rustup"}]);
    spec["containers"][0]["readinessProbe"] =
        json!({"exec":{"command":["/bin/sh","-c","test -f /tmp/old-ready"]}});
    spec["initContainers"] = json!([{
        "name":"workspace-bootstrap", "image":"example/workspace:1",
        "command":["/bin/sh", "-c", "echo old-init"]
    }]);
    let name = old["metadata"]["name"].as_str().unwrap();
    let path = format!(
        "/apis/apps/v1/namespaces/{}/statefulsets/{name}",
        workspace.runtime.namespace()
    );
    mock.objects.lock().unwrap().insert(path, old);
}

fn input_paths(coordinator: &KubernetesCoordinator, workspace: &Workspace) -> Vec<String> {
    let names = coordinator
        .builder
        .runtime_names(workspace)
        .unwrap()
        .resources;
    let namespace = workspace.runtime.namespace();
    [
        ("secrets", names.environment_secret),
        ("secrets", names.files_secret),
        ("configmaps", names.environment_config_map),
        ("configmaps", names.files_config_map),
    ]
    .into_iter()
    .map(|(kind, name)| format!("/api/v1/namespaces/{namespace}/{kind}/{name}"))
    .collect()
}

fn assert_only_inputs(mock: &GateMock, expected: &[String]) {
    let mut patches = mock
        .patches
        .lock()
        .unwrap()
        .iter()
        .map(|(path, _)| path.clone())
        .collect::<Vec<_>>();
    patches.sort();
    let mut expected = expected.to_vec();
    expected.sort();
    assert_eq!(patches, expected);
    assert!(mock.events.lock().unwrap().iter().all(|(method, path)| {
        (method == "GET" && path.contains("/statefulsets/"))
            || ((method == "GET" || method == "PATCH" || method == "APPLIED")
                && expected.contains(path))
    }));
}

#[tokio::test]
async fn injection_job_updates_four_inputs_without_reconciling_renderer_drift_or_state() {
    let database = Database::connect("sqlite::memory:", "public-a".parse().unwrap())
        .await
        .unwrap();
    let workspace = seed(&database).await;
    let cipher =
        EnvelopeCipher::from_base64(&EnvelopeCipher::generate_base64_key().unwrap()).unwrap();
    let text = "catalog instruction\n".repeat(8192);
    database
        .replace_injection(
            &cipher,
            InjectionScopeRef {
                scope: InjectionScope::User,
                scope_id: workspace.owner_id,
            },
            InjectionItem {
                key: "catalog".into(),
                kind: InjectionKind::ConfigFile,
                target: "/workspace/catalog.json".into(),
                value: InjectionValue::Utf8(text.clone()),
                sensitive: false,
                locked: false,
                version: 0,
                file_mode: None,
                owner: None,
                group: None,
                template_selector: None,
                labels: BTreeMap::new(),
            },
            workspace.owner_id,
            3,
        )
        .await
        .unwrap();
    let mock = Arc::new(GateMock::default());
    let coordinator = coordinator(mock.clone());
    seed_workload(&mock, &coordinator, &workspace);
    let before = mock.objects.lock().unwrap().clone();
    let paths = input_paths(&coordinator, &workspace);
    let handler = WorkspaceReconcileHandler::new(
        database.clone(),
        cipher,
        coordinator.builder.clone(),
        coordinator.clone(),
    );
    let mut injection_job = job(&workspace);
    // A checkpoint on an old attempt must not redirect an injection job to readiness reads.
    injection_job.payload["reconciled_generation"] = json!(workspace.generation);
    injection_job.payload["reconciled_at"] = json!(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );
    handler.handle(&injection_job).await.unwrap();
    assert_only_inputs(&mock, &paths);
    for (path, value) in before {
        assert_eq!(mock.objects.lock().unwrap().get(&path), Some(&value));
    }
    assert!(
        mock.object("-files-config").unwrap()["data"]
            .as_object()
            .unwrap()
            .values()
            .any(|value| value == &json!(text))
    );
    // Even terminal failure must not turn an input update into a lifecycle observation.
    handler
        .on_terminal_failure(&injection_job, 4)
        .await
        .unwrap();
    assert_eq!(
        database.get_workspace(workspace.id).await.unwrap(),
        workspace
    );
    let Database::Sqlite { pool, .. } = &database else {
        unreachable!()
    };
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM workspace_ssh_identities")
            .fetch_one(pool)
            .await
            .unwrap(),
        0
    );

    // An explicit restart still follows the full reconcile path, applying renderer changes.
    let restarting = database
        .request_workspace_action(
            workspace.id,
            WorkspaceAction::Restart,
            workspace.owner_id,
            5,
        )
        .await
        .unwrap();
    let mut restart_job = job(&restarting);
    restart_job.payload = json!({"generation":restarting.generation});
    assert!(matches!(
        handler.handle(&restart_job).await,
        Err(JobHandlerError::PendingWithCheckpoint { .. })
    ));
    assert!(
        mock.patches
            .lock()
            .unwrap()
            .iter()
            .any(|(path, _)| path.contains("/statefulsets/"))
    );
    let names = coordinator
        .builder
        .runtime_names(&workspace)
        .unwrap()
        .resources;
    assert!(
        mock.object(&format!("/configmaps/{}", names.workspace_config))
            .is_some()
    );

    for state in [WorkspaceState::Deleting, WorkspaceState::Deleted] {
        sqlx::query("UPDATE workspaces SET state = ?1 WHERE id = ?2")
            .bind(if state == WorkspaceState::Deleting {
                "deleting"
            } else {
                "deleted"
            })
            .bind(workspace.id.to_string())
            .execute(pool)
            .await
            .unwrap();
        mock.events.lock().unwrap().clear();
        handler.handle(&injection_job).await.unwrap();
        assert!(mock.events.lock().unwrap().is_empty());
        assert_eq!(
            database.get_workspace(workspace.id).await.unwrap().state,
            state
        );
    }
}

#[tokio::test]
async fn injection_only_apply_preserves_existing_transport_and_rejects_unknown_encoding() {
    for (gate, encoding, compressed, valid) in [
        (false, None, false, true),
        (false, Some("unknown"), false, true),
        (true, None, false, true),
        (true, Some("dictionary-v1"), true, true),
        (true, Some("unknown"), false, false),
    ] {
        let mut workspace = workspace(workspace_id());
        workspace.state = WorkspaceState::Ready;
        let mock = Arc::new(GateMock::default());
        let coordinator = coordinator(mock.clone());
        seed_workload(&mock, &coordinator, &workspace);
        {
            let mut objects = mock.objects.lock().unwrap();
            let old = objects.values_mut().next().unwrap();
            if gate {
                old["metadata"]["annotations"]["workspace.memeloop.dev/startup-gate"] =
                    json!("existing-startup-gate");
            }
            if let Some(encoding) = encoding {
                old["metadata"]["annotations"]["workspace.memeloop.dev/file-encoding"] =
                    json!(encoding);
            }
        }
        let before = mock.objects.lock().unwrap().clone();
        let mut injections = coordinator.builder.build(&workspace).unwrap().injections;
        let data = injections.file_config_map.data.as_mut().unwrap();
        let text = "repeated catalog line\n".repeat(8192);
        data.insert("catalog".into(), text.clone());
        data.insert(
            "workspace-files.json".into(),
            json!({"version":1,"files":[{
                "object":"config_map","kind":"config_file","data_key":"catalog",
                "target":"/workspace/catalog.json","mode":416
            }]})
            .to_string(),
        );
        let result = coordinator
            .reconcile_injections(&workspace, &injections)
            .await;
        if valid {
            result.unwrap();
            assert_only_inputs(&mock, &input_paths(&coordinator, &workspace));
            let files = mock.object("-files-config").unwrap();
            let manifest: Value =
                serde_json::from_str(files["data"]["workspace-files.json"].as_str().unwrap())
                    .unwrap();
            if compressed {
                assert_eq!(manifest["files"][0]["encoding"], "dictionary_v1");
                assert!(files["data"]["catalog"].as_str().unwrap().len() < text.len() / 2);
            } else {
                assert_eq!(files["data"]["catalog"], text);
            }
        } else {
            assert!(matches!(
                result,
                Err(ReconcileError::RuntimeIdentityMismatch)
            ));
            assert!(mock.writes.lock().unwrap().is_empty());
        }
        for (path, value) in before {
            assert_eq!(mock.objects.lock().unwrap().get(&path), Some(&value));
        }
    }
}

#[tokio::test]
async fn injection_updates_do_not_create_absent_or_terminating_workloads() {
    for terminating in [false, true] {
        let mut workspace = workspace(workspace_id());
        workspace.state = WorkspaceState::Ready;
        let mock = Arc::new(GateMock::default());
        let coordinator = coordinator(mock.clone());
        if terminating {
            seed_workload(&mock, &coordinator, &workspace);
            let mut objects = mock.objects.lock().unwrap();
            objects.values_mut().next().unwrap()["metadata"]["deletionTimestamp"] =
                json!("2026-10-27T00:00:00Z");
        }
        let injections = coordinator.builder.build(&workspace).unwrap().injections;
        coordinator
            .reconcile_injections(&workspace, &injections)
            .await
            .unwrap();
        assert!(mock.writes.lock().unwrap().is_empty());
        assert_eq!(mock.events.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn injection_updates_keep_workload_and_input_ownership_guards() {
    let mut workspace = workspace(workspace_id());
    workspace.state = WorkspaceState::Ready;
    let probe = coordinator(Arc::new(GateMock::default()));
    let mut paths = input_paths(&probe, &workspace);
    paths.push(format!(
        "/apis/apps/v1/namespaces/{}/statefulsets/w-8000000000000001",
        workspace.runtime.namespace()
    ));
    for path in paths {
        let mock = Arc::new(GateMock::default());
        let coordinator = coordinator(mock.clone());
        seed_workload(&mock, &coordinator, &workspace);
        let foreign = json!({
            "apiVersion":"v1", "kind":"PartialObjectMetadata", "metadata":{
                "name":path.rsplit('/').next().unwrap(),
                "labels":ownership_labels("another-installation", workspace.id)
            }
        });
        if path.contains("/statefulsets/") {
            let mut objects = mock.objects.lock().unwrap();
            objects.get_mut(&path).unwrap()["metadata"]["labels"] =
                foreign["metadata"]["labels"].clone();
        } else {
            mock.objects.lock().unwrap().insert(path.clone(), foreign);
        }
        let before = mock.objects.lock().unwrap().get(&path).cloned();
        let injections = coordinator.builder.build(&workspace).unwrap().injections;
        assert!(matches!(
            coordinator
                .reconcile_injections(&workspace, &injections)
                .await,
            Err(ReconcileError::Ownership(_))
        ));
        assert_eq!(mock.objects.lock().unwrap().get(&path), before.as_ref());
        assert!(
            !mock
                .writes
                .lock()
                .unwrap()
                .iter()
                .any(|(written, _)| written == &path)
        );
        if path.contains("/statefulsets/") {
            assert!(mock.writes.lock().unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn input_apply_errors_are_returned_and_retry_never_applies_the_workload() {
    for code in [409, 500] {
        let mut workspace = workspace(workspace_id());
        workspace.state = WorkspaceState::Ready;
        let mock = Arc::new(GateMock::default());
        let original = coordinator(mock.clone());
        seed_workload(&mock, &original, &workspace);
        let before = mock.object("/statefulsets/w-8000000000000001");
        let fail_once = Arc::new(AtomicBool::new(true));
        let service = service_fn({
            let mock = mock.clone();
            move |request: Request<kube::client::Body>| {
                let mock = mock.clone();
                let fail_once = fail_once.clone();
                async move {
                    if request.method() == Method::PATCH
                        && request.uri().path().ends_with("-files-config")
                        && fail_once.swap(false, Ordering::SeqCst)
                    {
                        return Ok::<_, Infallible>(json_response(
                            StatusCode::from_u16(code).unwrap(),
                            json!({"apiVersion":"v1","kind":"Status","status":"Failure",
                                "code":code,"reason":"Conflict","message":"input apply failed"}),
                        ));
                    }
                    Ok::<_, Infallible>(mock.respond(request).await)
                }
            }
        });
        let coordinator =
            KubernetesCoordinator::new(kube::Client::new(service, "default"), original.builder);
        let injections = coordinator.builder.build(&workspace).unwrap().injections;
        assert!(matches!(
            coordinator.reconcile_injections(&workspace, &injections).await,
            Err(ReconcileError::Kubernetes(kube::Error::Api(error))) if error.code == code
        ));
        mock.patches.lock().unwrap().clear();
        coordinator
            .reconcile_injections(&workspace, &injections)
            .await
            .unwrap();
        assert_only_inputs(&mock, &input_paths(&coordinator, &workspace));
        assert_eq!(mock.object("/statefulsets/w-8000000000000001"), before);
    }
}
