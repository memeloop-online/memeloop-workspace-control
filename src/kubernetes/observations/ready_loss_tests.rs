use super::*;
use crate::{
    auth::ApiKeyScope,
    crypto::EnvelopeCipher,
    jobs::{JobHandler, JobWorker, WorkspaceReconcileHandler},
    kubernetes::{InternetEgressConfig, KubernetesCoordinator, ResourceBuilder},
    quota::Resources,
    storage::{ClaimedJob, CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate},
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::{AccessMode, Workspace, WorkspaceAction, WorkspaceObservation, WorkspaceState},
};
use axum::{
    body::Body,
    http::{Method, Request, Response, StatusCode},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    convert::Infallible,
    sync::atomic::{AtomicBool, Ordering},
};
use tower::service_fn;

const INSTALLATION: &str = "ready-loss-test";

async fn seed(database: &Database) -> Workspace {
    let now = unix_now();
    database.migrate().await.unwrap();
    database
        .upsert_image_policy("example/workspace:1", true, now)
        .await
        .unwrap();
    let owner = database
        .create_user_with_initial_key(
            "Ready loss",
            "ready-loss-token-000000000000000000000000",
            true,
            ApiKeyScope::initial_key_defaults(true),
            now + 3600,
            now,
        )
        .await
        .unwrap()
        .user_id;
    let org = database
        .create_organization(
            CreateOrganization {
                name: "Ready loss".into(),
                owner_user_id: owner,
            },
            now,
        )
        .await
        .unwrap();
    let spec = WorkspaceTemplateSpec::standard(
        "example/workspace:1",
        AccessMode::Internal,
        Resources {
            cpu_millis: 1000,
            memory_mib: 2048,
            gpu_count: 0,
            disk_gib: 20,
        },
    );
    let template = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(org.id),
                yaml: WorkspaceTemplateDocument::new("Ready loss", spec)
                    .to_yaml()
                    .unwrap(),
            },
            true,
            now,
        )
        .await
        .unwrap();
    let workspace = database
        .create_workspace(
            CreateWorkspace {
                organization_id: org.id,
                owner_id: owner,
                name: "Ready loss".into(),
                template_id: template.id,
                resources: None,
                temporary_storage_gib: None,
                organization_injection_refs: None,
                user_injection_refs: None,
            },
            true,
            owner,
            now,
        )
        .await
        .unwrap();
    let job = database
        .claim_job("seed", now, Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    database.complete_job(job.id, "seed", now).await.unwrap();
    database
        .record_workspace_observation(
            workspace.id,
            workspace.generation,
            WorkspaceObservation::Ready,
            owner,
            now,
        )
        .await
        .unwrap()
        .unwrap()
}

fn builder() -> ResourceBuilder {
    ResourceBuilder {
        installation_id: INSTALLATION.parse().unwrap(),
        ttyd_image: "example/ttyd:1".into(),
        buildkit_image: "moby/buildkit:test".into(),
        ttyd_mtls: None,
        higress_namespace: "higress-system".into(),
        higress_pod_labels: BTreeMap::new(),
        higress_source_cidrs: Vec::new(),
        internet_egress: Some(
            InternetEgressConfig::new(
                "kube-system".into(),
                BTreeMap::from([("k8s-app".into(), "kube-dns".into())]),
                Vec::new(),
            )
            .unwrap(),
        ),
        jump_host_namespace: "access".into(),
        jump_host_pod_labels: BTreeMap::new(),
        storage_class_name: None,
        scratch_storage_class_name: "local-scratch".into(),
        web_shell_domain: None,
        port_mapping_domain: None,
        higress_gateway_name: "higress".into(),
        higress_https_section_name: "https".into(),
        internal_ssh_node_port_enabled: false,
    }
}

struct Runtime {
    set: Value,
    pod: Mutex<Option<Value>>,
    fail: AtomicBool,
}

impl Runtime {
    fn new(workspace: &Workspace, builder: &ResourceBuilder) -> Self {
        let mut set = serde_json::to_value(builder.build(workspace).unwrap().stateful_set).unwrap();
        set["metadata"]["uid"] = json!("set-current");
        set["metadata"]["generation"] = json!(1);
        // This deliberately remains stale Ready throughout the replacement Pod's Pending phase.
        set["status"] = json!({"observedGeneration":1,"replicas":1,"readyReplicas":1,"updatedReplicas":1,
            "currentRevision":"revision-current","updateRevision":"revision-current"});
        let mut metadata = set["spec"]["template"]["metadata"].clone();
        metadata["name"] = json!(format!("{}-0", set["metadata"]["name"].as_str().unwrap()));
        metadata["namespace"] = json!(WORKSPACE_NAMESPACE);
        metadata["uid"] = json!("replacement-pod");
        metadata["labels"]["controller-revision-hash"] = json!("revision-current");
        metadata["ownerReferences"] = json!([{"apiVersion":"apps/v1","kind":"StatefulSet",
            "name":set["metadata"]["name"],"uid":"set-current","controller":true}]);
        Self {
            set,
            pod: Mutex::new(Some(
                json!({"apiVersion":"v1","kind":"Pod","metadata":metadata,"status":{"phase":"Pending"}}),
            )),
            fail: AtomicBool::new(false),
        }
    }

    fn client(self: &Arc<Self>) -> kube::Client {
        let runtime = self.clone();
        kube::Client::new(
            service_fn(move |request: Request<kube::client::Body>| {
                let runtime = runtime.clone();
                async move {
                    assert_eq!(
                        request.method(),
                        Method::GET,
                        "runtime observation must never apply/delete resources"
                    );
                    let (status, body) = if runtime.fail.load(Ordering::SeqCst) {
                        (
                            StatusCode::SERVICE_UNAVAILABLE,
                            json!({"apiVersion":"v1","kind":"Status","status":"Failure","reason":"ServiceUnavailable","message":"injected read failure","code":503}),
                        )
                    } else if request.uri().path().contains("/statefulsets/") {
                        (StatusCode::OK, runtime.set.clone())
                    } else {
                        assert!(request.uri().path().contains("/pods/"));
                        runtime.pod.lock().unwrap().clone().map(|pod| (StatusCode::OK, pod)).unwrap_or_else(||
                        (StatusCode::NOT_FOUND, json!({"apiVersion":"v1","kind":"Status","status":"Failure","reason":"NotFound","message":"missing","code":404})))
                    };
                    Ok::<_, Infallible>(
                        Response::builder()
                            .status(status)
                            .header("content-type", "application/json")
                            .body(Body::from(body.to_string()))
                            .unwrap(),
                    )
                }
            }),
            "default",
        )
    }
}

fn worker(
    database: &Database,
    observer: &WorkspaceObservations,
    runtime: &Arc<Runtime>,
) -> JobWorker<WorkspaceReconcileHandler> {
    let cipher =
        EnvelopeCipher::from_base64(&EnvelopeCipher::generate_base64_key().unwrap()).unwrap();
    let handler = WorkspaceReconcileHandler::new(
        database.clone(),
        cipher,
        builder(),
        KubernetesCoordinator::new(runtime.client(), builder()),
    )
    .with_observations(observer.clone());
    JobWorker::new(
        database.clone(),
        Arc::new(handler),
        "ready-loss-worker".into(),
    )
}

async fn exercise(database: Database) {
    let workspace = seed(&database).await;
    let runtime = Arc::new(Runtime::new(&workspace, &builder()));
    let observer = WorkspaceObservations::new(database.clone(), INSTALLATION.parse().unwrap());
    {
        let mut state = observer.state.lock().unwrap();
        StatefulSet::update(
            &mut state,
            serde_json::from_value(runtime.set.clone()).unwrap(),
            INSTALLATION,
            false,
        );
        // The old Ready Pod disappeared. A matching replacement remains Pending.
        let mut old = runtime.pod.lock().unwrap().clone().unwrap();
        old["metadata"]["uid"] = json!("old-ready-pod");
        old["status"] = json!({"phase":"Running","conditions":[{"type":"Ready","status":"True"}]});
        Pod::update(
            &mut state,
            serde_json::from_value(old).unwrap(),
            INSTALLATION,
            false,
        );
    }
    let mut rv = None;
    let mut foreign = runtime.pod.lock().unwrap().clone().unwrap();
    foreign["metadata"]["labels"][OWNER_INSTALLATION_LABEL] = json!("another-installation");
    observer
        .apply_event(
            WatchEvent::Modified(serde_json::from_value::<Pod>(foreign).unwrap()),
            &mut rv,
        )
        .await;
    assert_eq!(database.job_counts().await.unwrap().pending, 0);
    observer
        .apply_event(
            WatchEvent::Modified(
                serde_json::from_value::<Pod>(runtime.pod.lock().unwrap().clone().unwrap())
                    .unwrap(),
            ),
            &mut rv,
        )
        .await;
    let counts = database.job_counts().await.unwrap();
    assert_eq!(counts.pending, 1);
    // A second resource stream and duplicate event coalesce with the first durable job.
    observer
        .apply_event(
            WatchEvent::Modified(
                serde_json::from_value::<StatefulSet>(runtime.set.clone()).unwrap(),
            ),
            &mut rv,
        )
        .await;
    assert_eq!(database.job_counts().await.unwrap().pending, 1);
    let worker = worker(&database, &observer, &runtime);
    assert!(worker.run_once(unix_now()).await.unwrap());
    let recovering = database.get_workspace(workspace.id).await.unwrap();
    assert_eq!(recovering.state, WorkspaceState::Starting);
    assert_eq!(recovering.generation, workspace.generation);
    let parked = database
        .list_parked_workspace_jobs(None, 256, unix_now())
        .await
        .unwrap();
    assert_eq!(parked.jobs.len(), 1);
    assert_eq!(parked.jobs[0].generation, workspace.generation);
    assert_eq!(parked.jobs[0].desired_replicas, 1);
    assert!(
        !worker.run_once(unix_now()).await.unwrap(),
        "no immediate repeated GET while Pending"
    );
    let ready = {
        let mut pod = runtime.pod.lock().unwrap();
        let pod = pod.as_mut().unwrap();
        pod["status"] = json!({"phase":"Running","conditions":[{"type":"Ready","status":"True"}]});
        pod.clone()
    };
    observer
        .apply_event(
            WatchEvent::Modified(serde_json::from_value::<Pod>(ready).unwrap()),
            &mut rv,
        )
        .await;
    assert!(
        worker.run_once(unix_now()).await.unwrap(),
        "Ready event must wake the parked job before its deadline"
    );
    let ready = database.get_workspace(workspace.id).await.unwrap();
    assert_eq!(ready.state, WorkspaceState::Ready);
    assert_eq!(ready.generation, workspace.generation);
    assert_eq!(database.job_counts().await.unwrap().pending, 0);

    // Kube read failure retains Ready instead of falsely reporting a lost Pod.
    runtime.fail.store(true, Ordering::SeqCst);
    let (a, b) = tokio::join!(
        database.enqueue_ready_workspace_observation(workspace.id, unix_now()),
        database.enqueue_ready_workspace_observation(workspace.id, unix_now())
    );
    assert_eq!(usize::from(a.unwrap()) + usize::from(b.unwrap()), 1);
    assert!(worker.run_once(unix_now()).await.unwrap());
    assert_eq!(
        database.get_workspace(workspace.id).await.unwrap().state,
        WorkspaceState::Ready
    );

    // A user Stop supersedes the parked observation. It must neither recover Ready
    // nor run deletion/reconcile writes when its old job becomes claimable.
    let stopping = database
        .request_workspace_action(
            workspace.id,
            WorkspaceAction::Stop,
            workspace.owner_id,
            unix_now(),
        )
        .await
        .unwrap();
    assert!(
        database
            .list_parked_workspace_jobs(None, 256, unix_now())
            .await
            .unwrap()
            .jobs
            .is_empty(),
        "old-generation parked marker is no longer eligible"
    );
    let stale = ClaimedJob {
        id: Uuid::now_v7(),
        kind: "reconcile_workspace".into(),
        workspace_id: Some(workspace.id),
        payload: json!({"generation":workspace.generation,"runtime_observation":true}),
        attempts: 1,
        lease_expires_at: unix_now() + 60,
    };
    let cipher =
        EnvelopeCipher::from_base64(&EnvelopeCipher::generate_base64_key().unwrap()).unwrap();
    let handler = WorkspaceReconcileHandler::new(
        database.clone(),
        cipher,
        builder(),
        KubernetesCoordinator::new(runtime.client(), builder()),
    );
    assert!(
        handler.handle(&stale).await.is_ok(),
        "stale observation must finish without GET or apply"
    );
    let deleting = database
        .request_workspace_action(
            workspace.id,
            WorkspaceAction::Delete,
            workspace.owner_id,
            unix_now(),
        )
        .await
        .unwrap();
    assert!(
        handler.handle(&stale).await.is_ok(),
        "observation must not enter the deletion reconciler"
    );
    assert_eq!(
        database.get_workspace(workspace.id).await.unwrap(),
        deleting
    );
    assert_eq!(stopping.generation, workspace.generation + 1);
    // Enqueue refuses stopped-intent workspaces, even when their cached Pod was Ready.
    assert!(
        !database
            .enqueue_ready_workspace_observation(workspace.id, unix_now())
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn loss_during_an_earlier_ready_job_is_rechecked_after_that_job_completes() {
    let database = Database::connect("sqlite::memory:", INSTALLATION.parse().unwrap())
        .await
        .unwrap();
    let workspace = seed(&database).await;
    let runtime = Arc::new(Runtime::new(&workspace, &builder()));
    let observer = WorkspaceObservations::new(database.clone(), INSTALLATION.parse().unwrap());
    assert!(
        database
            .enqueue_ready_workspace_observation(workspace.id, unix_now())
            .await
            .unwrap()
    );
    let earlier = database
        .claim_job("earlier-ready-read", unix_now(), Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    let pending = runtime.pod.lock().unwrap().clone().unwrap();
    observer
        .apply_event(
            WatchEvent::Modified(serde_json::from_value::<Pod>(pending).unwrap()),
            &mut None,
        )
        .await;
    assert!(
        observer
            .state
            .lock()
            .unwrap()
            .ready_checks
            .contains_key(&workspace.id)
    );
    assert_eq!(database.job_counts().await.unwrap().pending, 0);
    // Its GET had already returned Ready before the loss, so it leaves the row Ready.
    database
        .complete_job(earlier.id, "earlier-ready-read", unix_now())
        .await
        .unwrap();
    observer.repair().await.unwrap();
    assert_eq!(database.job_counts().await.unwrap().pending, 1);
    assert!(
        worker(&database, &observer, &runtime)
            .run_once(unix_now())
            .await
            .unwrap()
    );
    assert_eq!(
        database.get_workspace(workspace.id).await.unwrap().state,
        WorkspaceState::Starting
    );
}

#[tokio::test]
async fn complete_relist_finds_ready_rows_with_no_cached_pod_or_statefulset() {
    let database = Database::connect("sqlite::memory:", INSTALLATION.parse().unwrap())
        .await
        .unwrap();
    let workspace = seed(&database).await;
    let observer = WorkspaceObservations::new(database.clone(), INSTALLATION.parse().unwrap());
    {
        let mut state = observer.state.lock().unwrap();
        state.pods_listed = true;
        state.sets_listed = true;
        state.ready_scan_pending = true;
    }
    observer.scan_ready_after_relist().await.unwrap();
    assert_eq!(database.job_counts().await.unwrap().pending, 1);
    observer.scan_ready_after_relist().await.unwrap();
    assert_eq!(database.job_counts().await.unwrap().pending, 1);
    let runtime = Arc::new(Runtime::new(&workspace, &builder()));
    *runtime.pod.lock().unwrap() = None;
    assert!(
        worker(&database, &observer, &runtime)
            .run_once(unix_now())
            .await
            .unwrap()
    );
    let recovering = database.get_workspace(workspace.id).await.unwrap();
    assert_eq!(recovering.state, WorkspaceState::Starting);
    assert_eq!(recovering.generation, workspace.generation);
}

#[tokio::test]
async fn sqlite_ready_loss_recovers_without_reapplying_or_changing_generation() {
    exercise(
        Database::connect("sqlite::memory:", INSTALLATION.parse().unwrap())
            .await
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn postgres_ready_loss_recovers_and_coalesces_concurrent_producers() {
    let Ok(url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        return;
    };
    let schema = format!("mwc_ready_loss_{}", Uuid::now_v7().simple());
    let admin = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let mut scoped = url::Url::parse(&url).unwrap();
    scoped
        .query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema}"));
    let database = Database::connect(scoped.as_str(), INSTALLATION.parse().unwrap())
        .await
        .unwrap();
    exercise(database.clone()).await;
    if let Database::Postgres { pool, .. } = database {
        pool.close().await;
    }
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
