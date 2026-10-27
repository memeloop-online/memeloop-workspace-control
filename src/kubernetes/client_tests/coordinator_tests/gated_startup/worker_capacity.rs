//! Finite worker/watch mechanism acceptance, not CSI or sustained creation throughput.
use super::*;
use crate::{
    auth::ApiKeyScope,
    crypto::EnvelopeCipher,
    jobs::{
        ControlPlaneJobHandler, JobHandler, JobHandlerError, JobWorker, WebhookDeliveryHandler,
        WorkspaceReconcileHandler,
    },
    kubernetes::{OWNER_INSTALLATION_LABEL, WorkspaceObservations},
    observability::Observability,
    quota::QuotaResources,
    storage::{
        ClaimedJob, CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate, Database,
        StorageError,
    },
    templates::WorkspaceTemplateDocument,
};
use axum::{Router, response::IntoResponse};
use futures::FutureExt;
use std::{io::Write, panic::AssertUnwindSafe, time::Instant};
use tokio::{
    net::TcpListener,
    sync::{Barrier, oneshot, watch},
};

const TOTAL: usize = 32;
const SLOTS: usize = 4;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sqlite_workers_park_and_shared_relist_completes_all_before_repair() {
    let directory = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        directory.path().join("workers.sqlite").display()
    );
    let result = run_case("sqlite", &url).await;
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn postgres_workers_park_and_shared_relist_completes_all_before_repair() {
    let Ok(url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        report(
            "postgres",
            false,
            &[json!({"phase":"postgres_unavailable"})],
        );
        return;
    };
    let schema = format!("mwc_worker_watch_{}", Uuid::now_v7().simple());
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let mut scoped = url::Url::parse(&url).unwrap();
    scoped
        .query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema}"));
    let result = run_case("postgres", scoped.as_str()).await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}

fn report(backend: &str, completed: bool, stages: &[Value]) {
    writeln!(std::io::stdout().lock(), "MWC_WORKER_WATCH_ACCEPTANCE {}",
        json!({"backend":backend,"completed":completed,"workspaces":TOTAL,"slots":SLOTS,"stages":stages})).unwrap();
}

struct Counted {
    inner: ControlPlaneJobHandler,
    active: AtomicUsize,
    peak: AtomicUsize,
    calls: AtomicUsize,
    registered: AtomicUsize,
    first_wave: Barrier,
}
impl JobHandler for Counted {
    async fn handle(&self, job: &ClaimedJob) -> Result<(), JobHandlerError> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        if self.calls.fetch_add(1, Ordering::SeqCst) < SLOTS {
            self.first_wave.wait().await;
        }
        let result = self.inner.handle(job).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        result
    }
    async fn on_checkpoint_deferred(&self, id: Uuid, now: i64) -> Result<(), StorageError> {
        self.inner.on_checkpoint_deferred(id, now).await?;
        self.registered.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct HttpState {
    mock: GateMock,
    verbs: Mutex<BTreeMap<String, usize>>,
    lists: AtomicUsize,
    pod_watches: AtomicUsize,
    set_watches: AtomicUsize,
    release_ready: Arc<Notify>,
}
impl Default for HttpState {
    fn default() -> Self {
        Self {
            mock: GateMock::default(),
            verbs: Mutex::default(),
            lists: AtomicUsize::new(0),
            pod_watches: AtomicUsize::new(0),
            set_watches: AtomicUsize::new(0),
            release_ready: Arc::new(Notify::new()),
        }
    }
}
impl HttpState {
    fn verbs(&self) -> BTreeMap<String, usize> {
        self.verbs.lock().unwrap().clone()
    }
    async fn respond(&self, request: Request<Body>) -> Response<Body> {
        let method = request.method().clone();
        *self
            .verbs
            .lock()
            .unwrap()
            .entry(method.to_string())
            .or_default() += 1;
        let path = request.uri().path();
        if method == Method::GET && (path.ends_with("/pods") || path.ends_with("/statefulsets")) {
            let query: BTreeMap<_, _> =
                url::form_urlencoded::parse(request.uri().query().unwrap_or("").as_bytes())
                    .into_owned()
                    .collect();
            assert_eq!(
                query["labelSelector"],
                format!("{OWNER_INSTALLATION_LABEL}=public-a")
            );
            let pods = path.ends_with("/pods");
            if query.contains_key("watch") {
                let number = if pods {
                    &self.pod_watches
                } else {
                    &self.set_watches
                }
                .fetch_add(1, Ordering::SeqCst);
                assert!(pods || number < 2, "unexpected StatefulSet watch reconnect");
                assert!(
                    !pods || number == 0,
                    "Pod watch must remain shared and open"
                );
                if !pods && number == 0 {
                    // The one expiry forces a single complete snapshot containing all Ready sets.
                    // No per-workspace watch/HTTP event is synthesized.
                    let release = self.release_ready.clone();
                    return Response::new(Body::from_stream(futures::stream::once(async move {
                        release.notified().await;
                        Ok::<_, std::io::Error>(axum::body::Bytes::from(format!(
                            "{}\n",
                            json!({"type":"ERROR","object":{"apiVersion":"v1","kind":"Status",
                            "status":"Failure","reason":"Expired","code":410,"message":"test expiry"}})
                        )))
                    })));
                }
                return Response::new(Body::from_stream(futures::stream::pending::<
                    Result<axum::body::Bytes, std::io::Error>,
                >()));
            }
            let items = if pods {
                vec![]
            } else {
                let sequence = self.lists.fetch_add(1, Ordering::SeqCst);
                assert!(
                    sequence < 2,
                    "only initial snapshot and one expired-watch relist"
                );
                if sequence == 0 {
                    vec![]
                } else {
                    self.mock
                        .objects
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|(path, _)| path.contains("/statefulsets/"))
                        .map(|(_, value)| value.clone())
                        .collect::<Vec<_>>()
                }
            };
            return json_response(
                StatusCode::OK,
                json!({"apiVersion":if pods {"v1"} else {"apps/v1"},
                "kind":if pods {"PodList"} else {"StatefulSetList"},"metadata":{"resourceVersion":"snapshot-1"},"items":items}),
            );
        }
        self.mock.respond(request).await
    }
    fn ready(&self) {
        let mut objects = self.mock.objects.lock().unwrap();
        let mut count = 0;
        let mut pods = Vec::new();
        for (path, value) in objects
            .iter_mut()
            .filter(|(path, _)| path.contains("/statefulsets/"))
        {
            value["metadata"]["generation"] = json!(1);
            value["metadata"]["resourceVersion"] = json!(format!("ready-{count}"));
            value["status"] = json!({"replicas":1,"readyReplicas":1,"updatedReplicas":1,
                "observedGeneration":1,"currentRevision":"ready","updateRevision":"ready"});
            assert!(path.starts_with("/apis/apps/v1/namespaces/"));
            let name = value["metadata"]["name"].as_str().unwrap();
            let namespace = value["metadata"]["namespace"].as_str().unwrap();
            let mut metadata = value["spec"]["template"]["metadata"].clone();
            metadata["name"] = json!(format!("{name}-0"));
            metadata["namespace"] = json!(namespace);
            metadata["uid"] = json!(format!("{name}-pod-uid"));
            metadata["labels"]["controller-revision-hash"] = json!("ready");
            metadata["ownerReferences"] = json!([{"apiVersion":"apps/v1","kind":"StatefulSet",
                "name":name,"uid":value["metadata"]["uid"],"controller":true}]);
            pods.push((
                format!("/api/v1/namespaces/{namespace}/pods/{name}-0"),
                json!({"apiVersion":"v1","kind":"Pod","metadata":metadata,
                    "status":{"phase":"Running","conditions":[{"type":"Ready","status":"True"}]}}),
            ));
            count += 1;
        }
        assert_eq!(count, TOTAL);
        objects.extend(pods);
    }
}

async fn seed(database: &Database) {
    let now = now();
    database
        .upsert_image_policy("example/workspace:1", true, now)
        .await
        .unwrap();
    let owner = database
        .create_user_with_initial_key(
            "Worker test",
            "worker-test-token-000000000000000000000",
            true,
            ApiKeyScope::initial_key_defaults(true),
            now + 3600,
            now,
        )
        .await
        .unwrap()
        .user_id;
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Workers".into(),
                owner_user_id: owner,
            },
            now,
        )
        .await
        .unwrap();
    let quota = QuotaResources {
        cpu_millis: 100_000,
        memory_mib: 100_000,
        gpu_count: 0,
        disk_gib: 10_000,
        temporary_storage_gib: 10_000,
    };
    database.set_user_quota(owner, quota, now).await.unwrap();
    database
        .set_organization_quota(organization.id, quota, now)
        .await
        .unwrap();
    let template = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(organization.id),
                yaml: WorkspaceTemplateDocument::new(
                    "Worker template",
                    workspace(workspace_id()).template,
                )
                .to_yaml()
                .unwrap(),
            },
            true,
            now,
        )
        .await
        .unwrap();
    for index in 0..TOTAL {
        database
            .create_workspace(
                CreateWorkspace {
                    organization_id: organization.id,
                    owner_id: owner,
                    name: format!("worker-{index}"),
                    template_id: template.id,
                    resources: None,
                    temporary_storage_gib: None,
                    use_longhorn: false,
                    organization_injection_refs: None,
                    user_injection_refs: None,
                },
                true,
                owner,
                now,
            )
            .await
            .unwrap();
    }
}

async fn queue(database: &Database) -> Value {
    let sql = "SELECT status, attempts, lease_owner, lease_expires_at, payload_json, available_at FROM jobs";
    type QueueJob = (String, i64, Option<String>, Option<i64>, String, i64);
    let (rows, leases, ready): (Vec<QueueJob>, i64, i64) = match database {
        Database::Sqlite { pool, .. } => (
            sqlx::query_as(sql).fetch_all(pool).await.unwrap(),
            sqlx::query_scalar("SELECT COUNT(*) FROM workspace_leases")
                .fetch_one(pool)
                .await
                .unwrap(),
            sqlx::query_scalar("SELECT COUNT(*) FROM workspaces WHERE state = 'ready'")
                .fetch_one(pool)
                .await
                .unwrap(),
        ),
        Database::Postgres { pool, .. } => (
            sqlx::query_as(sql).fetch_all(pool).await.unwrap(),
            sqlx::query_scalar("SELECT COUNT(*) FROM workspace_leases")
                .fetch_one(pool)
                .await
                .unwrap(),
            sqlx::query_scalar("SELECT COUNT(*) FROM workspaces WHERE state = 'ready'")
                .fetch_one(pool)
                .await
                .unwrap(),
        ),
    };
    json!({"total":rows.len(),"pending":rows.iter().filter(|r|r.0=="pending").count(),
        "completed":rows.iter().filter(|r|r.0=="completed").count(),"attempts_zero":rows.iter().filter(|r|r.1==0).count(),
        "attempts_one":rows.iter().filter(|r|r.1==1).count(),"job_leases":rows.iter().filter(|r|r.2.is_some()||r.3.is_some()).count(),
        "workspace_leases":leases,"ready":ready,"checkpointed":rows.iter().filter(|r| serde_json::from_str::<Value>(&r.4).unwrap()["reconciled_generation"]==1).count(),
        "earliest_repair":rows.iter().map(|r|r.5).min()})
}
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

async fn run_case(backend: &str, url: &str) -> std::thread::Result<()> {
    let database = Database::connect(url, "public-a".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
        .set_job_worker_concurrency(SLOTS as u16, now())
        .await
        .unwrap();
    seed(&database).await;
    let http = Arc::new(HttpState::default());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = kube::Config::new(
        format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap(),
    );
    let finite = crate::kubernetes::request_client(config.clone()).unwrap();
    let stream = crate::kubernetes::watch_client(config).unwrap();
    let app = Router::new().fallback({
        let http = http.clone();
        move |request: Request<Body>| {
            let http = http.clone();
            async move { http.respond(request).await.into_response() }
        }
    });
    let (stop_server, server_shutdown) = oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = server_shutdown.await;
            })
            .await
            .unwrap()
    });
    let observations = WorkspaceObservations::new(database.clone(), "public-a".parse().unwrap());
    let builder = coordinator(Arc::new(GateMock::default())).builder;
    let coordinator = KubernetesCoordinator::new(finite.clone(), builder.clone());
    let cipher =
        EnvelopeCipher::from_base64(&EnvelopeCipher::generate_base64_key().unwrap()).unwrap();
    let handler = Arc::new(Counted {
        inner: ControlPlaneJobHandler::new(
            Some(
                WorkspaceReconcileHandler::new(
                    database.clone(),
                    cipher.clone(),
                    builder,
                    coordinator,
                )
                .with_observations(observations.clone()),
            ),
            WebhookDeliveryHandler::new(database.clone(), cipher, Observability::default())
                .unwrap(),
        ),
        active: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
        calls: AtomicUsize::new(0),
        registered: AtomicUsize::new(0),
        first_wave: Barrier::new(SLOTS),
    });
    let (shutdown, stopped) = watch::channel(false);
    let observation =
        tokio::spawn(observations.run_until_shutdown(finite, stream, stopped.clone()));
    let worker = JobWorker::new(database.clone(), handler.clone(), "capacity-worker".into());
    let pool = tokio::spawn(async move { worker.run_pool_until_shutdown(stopped).await.unwrap() });
    let started = Instant::now();
    let mut stages = Vec::new();
    let outcome = AssertUnwindSafe(async {
        tokio::time::timeout(Duration::from_secs(45), async {
            while http.pod_watches.load(Ordering::SeqCst) != 1 || http.set_watches.load(Ordering::SeqCst) != 1 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            loop {
                let state=queue(&database).await;
                if state["pending"]==TOTAL && state["checkpointed"]==TOTAL && state["job_leases"]==0 && state["workspace_leases"]==0 && handler.registered.load(Ordering::SeqCst)==TOTAL {break;}
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert_eq!(handler.active.load(Ordering::SeqCst),0);
            assert_eq!(handler.peak.load(Ordering::SeqCst),SLOTS);
            assert_eq!(handler.calls.load(Ordering::SeqCst),TOTAL);
            assert_eq!(http.pod_watches.load(Ordering::SeqCst),1);
            assert_eq!(http.set_watches.load(Ordering::SeqCst),1);
            let parked=queue(&database).await;
            assert_eq!(parked["attempts_zero"],TOTAL);
            assert_eq!(parked["ready"],0);
            let verbs=http.verbs();
            let writes=http.mock.writes.lock().unwrap().len();
            let park_seconds=started.elapsed().as_secs_f64();
            tokio::time::sleep(Duration::from_millis(300)).await;
            assert_eq!(queue(&database).await,parked);
            assert_eq!(http.verbs(),verbs,"parked jobs must release slots without polling Kubernetes");
            assert_eq!(handler.active.load(Ordering::SeqCst),0);
            stages.push(json!({"phase":"parked","seconds":park_seconds,"quiet_millis":300,"queue":parked,"verbs":verbs,"handler_peak":handler.peak.load(Ordering::SeqCst)}));
            http.ready();
            let released=Instant::now();
            http.release_ready.notify_one();
            loop {
                let state=queue(&database).await;
                if state["completed"]==TOTAL && state["ready"]==TOTAL && state["workspace_leases"]==0 {break;}
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let final_state=queue(&database).await;
            assert_eq!(final_state["attempts_one"],TOTAL);
            assert_eq!(final_state["job_leases"],0);
            assert!(now()<parked["earliest_repair"].as_i64().unwrap(),"Ready must precede the original repair deadline");
            assert_eq!(http.mock.writes.lock().unwrap().len(),writes,"checkpoint resumes must not reapply resources");
            assert_eq!(http.verbs().get("PATCH"),verbs.get("PATCH"));
            assert_eq!(http.verbs().get("POST"),verbs.get("POST"));
            assert_eq!(http.lists.load(Ordering::SeqCst),2);
            assert_eq!(http.pod_watches.load(Ordering::SeqCst),1);
            assert_eq!(handler.calls.load(Ordering::SeqCst),2*TOTAL);
            stages.push(json!({"phase":"completed","seconds":started.elapsed().as_secs_f64(),"ready_after_release_seconds":released.elapsed().as_secs_f64(),"queue":final_state,"verbs":http.verbs(),"handler_peak":handler.peak.load(Ordering::SeqCst),"pod_watches":http.pod_watches.load(Ordering::SeqCst),"set_watches":http.set_watches.load(Ordering::SeqCst),"statefulset_lists":http.lists.load(Ordering::SeqCst)}));
        }).await.expect("worker/watch acceptance exceeded its 45 second budget");
    }).catch_unwind().await;
    shutdown.send(true).unwrap();
    // Both stop before the HTTP server: shutdown must cancel open streaming bodies.
    tokio::time::timeout(Duration::from_secs(5), async {
        pool.await.unwrap();
        observation.await.unwrap();
    })
    .await
    .expect("background worker/observer shutdown");
    stop_server.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("HTTP graceful shutdown")
        .unwrap();
    match &database {
        Database::Sqlite { pool, .. } => pool.close().await,
        Database::Postgres { pool, .. } => pool.close().await,
    };
    report(backend, outcome.is_ok(), &stages);
    outcome
}
