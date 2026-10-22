//! Finite HTTP acceptance and durable enqueue evidence, not Pod startup capacity.
use super::*;
use std::{
    collections::HashSet,
    convert::Infallible,
    io::Write,
    panic::AssertUnwindSafe,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

use futures::FutureExt;
use memeloop_workspace_control::{
    kubernetes::OWNER_INSTALLATION_LABEL, storage::CreateOrganization,
};
use sqlx::Row;
use tokio::{sync::Barrier, task::JoinSet};
use tower::service_fn;

const TOTAL: usize = 32;
const NAMESPACE: &str = "memeloop-workspace-control";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sqlite_http_acceptance_and_quota_preserve_durable_queue() {
    let directory = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        directory.path().join("acceptance.sqlite").display()
    );
    let outcome = run_case("sqlite", &url).await;
    if let Err(failure) = outcome {
        std::panic::resume_unwind(failure);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn postgres_http_acceptance_and_quota_preserve_durable_queue() {
    let Ok(url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        write_report(
            "postgres",
            &[json!({"phase": "postgres_unavailable"})],
            false,
        );
        eprintln!("skipping PostgreSQL acceptance test: MWC_TEST_POSTGRES_URL is not set");
        return;
    };
    let schema = format!("mwc_create_acceptance_{}", Uuid::now_v7().simple());
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("connect CI PostgreSQL");
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .expect("create isolated acceptance schema");
    let mut scoped = url::Url::parse(&url).unwrap();
    scoped
        .query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema}"));
    let outcome = run_case("postgres", scoped.as_str()).await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop isolated acceptance schema");
    admin.close().await;
    if let Err(failure) = outcome {
        std::panic::resume_unwind(failure);
    }
}

async fn run_case(backend: &str, url: &str) -> std::thread::Result<()> {
    let mut stages = vec![json!({"phase": "initializing"})];
    let outcome = AssertUnwindSafe(async {
        tokio::time::timeout(Duration::from_secs(120), exercise(url, &mut stages))
            .await
            .expect("bounded HTTP acceptance experiment timed out");
    })
    .catch_unwind()
    .await;
    write_report(backend, &stages, outcome.is_ok());
    outcome
}

async fn exercise(url: &str, stages: &mut Vec<Value>) {
    let database = Database::connect(url, "creation-acceptance".parse().unwrap())
        .await
        .expect("connect acceptance database");
    let (state, owner) = test_state(database.clone(), url).await;
    let mut http = AcceptanceHttp::start(state).await;
    let mut accepted_ids = HashSet::new();
    for parallelism in [1, 8, 32] {
        let payload = scope_payload(&database, owner, &format!("parallel-{parallelism}"), 64).await;
        exercise_batch(
            &database,
            &http,
            payload,
            parallelism,
            &mut accepted_ids,
            stages,
        )
        .await;
    }
    let quota_owner = database
        .authenticate(OUTSIDER_TOKEN)
        .await
        .unwrap()
        .unwrap()
        .user_id;
    database
        .set_user_quota(quota_owner, quota(5), 2)
        .await
        .unwrap();
    let payload = scope_payload(&database, quota_owner, "quota", 8).await;
    exercise_batch(&database, &http, payload, 32, &mut accepted_ids, stages).await;
    let before = queue_evidence(&database, &accepted_ids).await;
    stages.push(json!({"phase": "before_reconnect", "queue": before}));
    http.stop().await;
    close_database(&database).await;
    let reconnected = Database::connect(url, database.installation_id().clone())
        .await
        .expect("reopen acceptance database");
    let after = queue_evidence(&reconnected, &accepted_ids).await;
    stages.push(json!({"phase": "reconnected", "queue": after}));
    close_database(&reconnected).await;
    assert_eq!(before, after, "durable queue changed on reconnect");
}

fn namespace_client(requests: Arc<AtomicUsize>) -> kube::Client {
    let service = service_fn(move |request: Request<kube::client::Body>| {
        let requests = requests.clone();
        async move {
            assert_eq!(request.method(), Method::GET);
            assert_eq!(
                request.uri().path(),
                format!("/api/v1/namespaces/{NAMESPACE}")
            );
            requests.fetch_add(1, Ordering::Relaxed);
            let value = json!({"apiVersion": "v1", "kind": "Namespace", "metadata": {
                "name": NAMESPACE,
                "labels": {(OWNER_INSTALLATION_LABEL): "creation-acceptance"}
            }});
            Ok::<_, Infallible>(
                axum::response::Response::builder()
                    .header("content-type", "application/json")
                    .body(Body::from(value.to_string()))
                    .unwrap(),
            )
        }
    });
    kube::Client::new(service, NAMESPACE)
}

struct AcceptanceHttp {
    url: String,
    client: reqwest::Client,
    namespace_requests: Arc<AtomicUsize>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl AcceptanceHttp {
    async fn start(mut state: AppState) -> Self {
        let namespace_requests = Arc::new(AtomicUsize::new(0));
        state.set_kubernetes_client(namespace_client(namespace_requests.clone()));
        let app = router(Arc::new(state));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/api/v1/workspaces",
            listener.local_addr().unwrap()
        );
        let (shutdown, stopped) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .expect("serve acceptance HTTP");
        });
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        Self {
            url,
            client,
            namespace_requests,
            shutdown: Some(shutdown),
            task,
        }
    }

    async fn stop(&mut self) {
        let _ = self.shutdown.take().unwrap().send(());
        tokio::time::timeout(Duration::from_secs(5), &mut self.task)
            .await
            .expect("stop acceptance HTTP")
            .expect("acceptance HTTP task failed");
    }
}

impl Drop for AcceptanceHttp {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn quota(slots: u64) -> QuotaResources {
    QuotaResources {
        cpu_millis: slots * 500,
        memory_mib: 100_000,
        gpu_count: 0,
        disk_gib: 10_000,
        temporary_storage_gib: 10_000,
    }
}

async fn scope_payload(database: &Database, owner: Uuid, name: &str, slots: u64) -> Value {
    let organization = database
        .create_organization(
            CreateOrganization {
                name: name.to_owned(),
                owner_user_id: owner,
            },
            2,
        )
        .await
        .unwrap();
    database
        .set_organization_quota(organization.id, quota(slots), 2)
        .await
        .unwrap();
    let template = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(organization.id),
                yaml: WorkspaceTemplateDocument::new(
                    "Acceptance template",
                    WorkspaceTemplateSpec::standard(
                        "registry.example/workspace:1",
                        AccessMode::Internal,
                        Resources {
                            cpu_millis: 500,
                            memory_mib: 128,
                            gpu_count: 0,
                            disk_gib: 1,
                        },
                    ),
                )
                .to_yaml()
                .unwrap(),
            },
            true,
            2,
        )
        .await
        .unwrap();
    json!({"organization_id": organization.id, "owner_id": owner,
        "name": name, "template_id": template.id})
}

struct PostResult {
    index: usize,
    status: u16,
    body: Vec<u8>,
    seconds: f64,
    complete: bool,
}

async fn post(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    payload: &Value,
    index: usize,
) -> PostResult {
    let started = Instant::now();
    let response = client
        .post(url)
        .bearer_auth(ADMIN_TOKEN)
        .header("Idempotency-Key", key)
        .json(payload)
        .send()
        .await;
    let (status, body, complete) = match response {
        Ok(response) => {
            let status = response.status().as_u16();
            match response.bytes().await {
                Ok(bytes) => (status, bytes.to_vec(), true),
                Err(_) => (status, Vec::new(), false),
            }
        }
        Err(_) => (0, Vec::new(), false),
    };
    PostResult {
        index,
        status,
        body,
        seconds: started.elapsed().as_secs_f64(),
        complete,
    }
}

async fn batch(
    database: &Database,
    http: &AcceptanceHttp,
    payload: &Value,
    parallelism: usize,
    prefix: &str,
) -> (Vec<PostResult>, Value, f64) {
    let barrier = Arc::new(Barrier::new(parallelism));
    let mut tasks = JoinSet::new();
    let started = Instant::now();
    for slot in 0..parallelism {
        let (client, url, payload, prefix, barrier) = (
            http.client.clone(),
            http.url.clone(),
            payload.clone(),
            prefix.to_owned(),
            barrier.clone(),
        );
        tasks.spawn(async move {
            barrier.wait().await;
            let mut results = Vec::new();
            for index in (slot..TOTAL).step_by(parallelism) {
                let mut request = payload.clone();
                request["name"] = json!(format!("{prefix}-{index}"));
                results
                    .push(post(&client, &url, &format!("{prefix}-{index}"), &request, index).await);
            }
            results
        });
    }
    let mut results = Vec::new();
    let mut pool = Vec::new();
    let mut interval = tokio::time::interval(Duration::from_millis(10));
    while !tasks.is_empty() {
        tokio::select! {
            completed = tasks.join_next() => results.extend(completed.unwrap().expect("HTTP batch task failed")),
            _ = interval.tick() => pool.push(pool_snapshot(database)),
        }
    }
    pool.push(pool_snapshot(database));
    (results, json!(pool), started.elapsed().as_secs_f64())
}

fn pool_snapshot(database: &Database) -> Value {
    match database {
        Database::Sqlite { pool, .. } => {
            json!({"max": 1, "size": pool.size(), "idle": pool.num_idle()})
        }
        Database::Postgres { pool, .. } => {
            json!({"max": 20, "size": pool.size(), "idle": pool.num_idle()})
        }
    }
}

async fn exercise_batch(
    database: &Database,
    http: &AcceptanceHttp,
    payload: Value,
    parallelism: usize,
    accepted_ids: &mut HashSet<String>,
    stages: &mut Vec<Value>,
) {
    let is_quota = payload["name"] == "quota";
    let prefix = format!("acceptance-{}", payload["name"].as_str().unwrap());
    let requests_before = http.namespace_requests.load(Ordering::Relaxed);
    let (results, pool, seconds) = batch(database, http, &payload, parallelism, &prefix).await;
    let accepted = results
        .iter()
        .filter(|result| result.status == 201 && result.complete)
        .count();
    let rejected = results
        .iter()
        .filter(|result| result.status == 422 && result.complete)
        .count();
    let mut timings: Vec<_> = results.iter().map(|result| result.seconds).collect();
    timings.sort_by(f64::total_cmp);
    stages.push(json!({"phase": if is_quota {"quota"} else {"acceptance"},
        "parallelism": parallelism, "attempted": results.len(), "accepted": accepted,
        "quota_rejected": rejected, "wall_seconds": seconds, "accepted_per_second": accepted as f64 / seconds,
        "latency_seconds": {"min": timings.first(), "p50": timings.get(15), "p95": timings.get(30), "max": timings.last()},
        "statuses": results.iter().map(|result| result.status).collect::<Vec<_>>(),
        "incomplete_responses": results.iter().filter(|result| !result.complete).count(),
        "pool_occupancy_snapshots_not_acquire_time": pool,
        "namespace_gets": http.namespace_requests.load(Ordering::Relaxed) - requests_before}));
    assert_eq!(results.len(), TOTAL);
    assert_eq!(accepted, if is_quota { 5 } else { TOTAL });
    assert_eq!(rejected, if is_quota { TOTAL - 5 } else { 0 });
    assert_eq!(
        http.namespace_requests.load(Ordering::Relaxed) - requests_before,
        TOTAL
    );
    for result in &results {
        let value: Value = serde_json::from_slice(&result.body).expect("decode create response");
        if result.status == 201 {
            assert!(accepted_ids.insert(value["workspace"]["id"].as_str().unwrap().to_owned()));
            assert_eq!(value["workspace"]["state"], "provisioning");
            assert_eq!(value["workspace"]["generation"], 1);
        } else {
            assert_eq!(value["error"]["code"], "quota_exceeded");
        }
    }
    let first = results.iter().find(|result| result.status == 201).unwrap();
    let queue = verify_replay(database, http, &payload, &prefix, first, accepted_ids).await;
    let stage = stages.last_mut().unwrap();
    stage["queue"] = queue;
    stage["idempotency_replay_and_conflict_verified"] = json!(true);
}

async fn verify_replay(
    database: &Database,
    http: &AcceptanceHttp,
    payload: &Value,
    prefix: &str,
    first: &PostResult,
    accepted_ids: &HashSet<String>,
) -> Value {
    let before = queue_evidence(database, accepted_ids).await;
    let requests_before = http.namespace_requests.load(Ordering::Relaxed);
    let mut request = payload.clone();
    request["name"] = json!(format!("{prefix}-{}", first.index));
    let key = format!("{prefix}-{}", first.index);
    let replay = post(&http.client, &http.url, &key, &request, first.index).await;
    assert!(
        replay.complete && replay.status == 201 && replay.body == first.body,
        "exact replay changed"
    );
    request["name"] = json!("conflicting-name");
    let conflict = post(&http.client, &http.url, &key, &request, first.index).await;
    assert!(
        conflict.complete && conflict.status == 409,
        "changed request did not conflict"
    );
    assert_eq!(
        requests_before,
        http.namespace_requests.load(Ordering::Relaxed)
    );
    assert_eq!(before, queue_evidence(database, accepted_ids).await);
    before
}

async fn queue_evidence(database: &Database, accepted_ids: &HashSet<String>) -> Value {
    macro_rules! inspect {
        ($pool:expr) => {{
            let installation = database.installation_id().as_str();
            let rows = sqlx::query("SELECT id, short_id, state, generation FROM workspaces WHERE installation_id = $1")
                .bind(installation).fetch_all($pool).await.expect("read accepted workspaces");
            let mut ids = HashSet::new();
            let mut short_ids = HashSet::new();
            for row in &rows {
                assert!(ids.insert(row.get::<String, _>("id")));
                assert!(short_ids.insert(row.get::<String, _>("short_id")));
                assert_eq!(row.get::<String, _>("state"), "provisioning");
                assert_eq!(row.get::<i64, _>("generation"), 1);
            }
            assert_eq!(&ids, accepted_ids);
            let jobs = sqlx::query("SELECT workspace_id, kind, payload_json, status, available_at, attempts, lease_owner, lease_expires_at FROM jobs WHERE installation_id = $1")
                .bind(installation).fetch_all($pool).await.expect("read persisted create jobs");
            let mut queued = HashSet::new();
            for job in &jobs {
                assert!(queued.insert(job.get::<String, _>("workspace_id")));
                assert_eq!(job.get::<String, _>("kind"), "reconcile_workspace");
                assert_eq!(job.get::<String, _>("status"), "pending");
                assert_eq!(job.get::<i64, _>("attempts"), 0);
                assert!(job.get::<Option<String>, _>("lease_owner").is_none());
                assert!(job.get::<Option<i64>, _>("lease_expires_at").is_none());
                assert!(job.get::<i64, _>("available_at") > 0);
                let payload: Value = serde_json::from_str(&job.get::<String, _>("payload_json")).unwrap();
                assert_eq!(payload, json!({"generation": 1}));
            }
            assert_eq!(queued, ids);
            let audits: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE installation_id = $1 AND action = 'workspace.create'")
                .bind(installation).fetch_one($pool).await.unwrap();
            let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE installation_id = $1 AND kind = 'workspace.state_changed'")
                .bind(installation).fetch_one($pool).await.unwrap();
            let complete: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM idempotency_keys WHERE installation_id = $1 AND status_code = 201")
                .bind(installation).fetch_one($pool).await.unwrap();
            let incomplete: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM idempotency_keys WHERE installation_id = $1 AND status_code = 0")
                .bind(installation).fetch_one($pool).await.unwrap();
            assert_eq!(audits as usize, ids.len());
            assert_eq!(events, audits);
            assert_eq!(complete, audits);
            assert_eq!(incomplete, 0);
            json!({"workspaces": rows.len(), "pending_jobs": jobs.len(), "create_audits": audits,
                "state_changed_events": events, "completed_idempotency": complete, "incomplete_idempotency": incomplete})
        }};
    }
    match database {
        Database::Sqlite { pool, .. } => inspect!(pool),
        Database::Postgres { pool, .. } => inspect!(pool),
    }
}

async fn close_database(database: &Database) {
    match database {
        Database::Sqlite { pool, .. } => pool.close().await,
        Database::Postgres { pool, .. } => pool.close().await,
    }
}

fn write_report(backend: &str, stages: &[Value], completed: bool) {
    let report = json!({"scope": "http_acceptance_and_durable_enqueue_only", "backend": backend,
        "completed": completed, "stages": stages});
    // Direct writes bypass libtest's println capture, including successful cases.
    writeln!(std::io::stdout().lock(), "MWC_CREATION_ACCEPTANCE {report}")
        .expect("write safe acceptance report");
}
