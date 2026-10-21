use super::*;

use axum::{
    Json, Router,
    body::Body,
    http::{StatusCode, Uri},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use sqlx::Connection;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::{net::TcpListener, sync::Notify, task::JoinHandle};

const INSTALLATION: &str = "http-observer-test";
type Requests = Arc<Mutex<Vec<(String, HashMap<String, String>)>>>;

struct Server {
    finite: kube::Client,
    stream: kube::Client,
    task: JoinHandle<()>,
}

impl Server {
    async fn start(router: Router) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let config = kube::Config::new(format!("http://{address}").parse().unwrap());
        let finite = crate::kubernetes::request_client(config.clone()).unwrap();
        let stream = crate::kubernetes::watch_client(config).unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self {
            finite,
            stream,
            task,
        }
    }

    fn run(&self, observer: WorkspaceObservations) -> (watch::Sender<bool>, JoinHandle<()>) {
        let (shutdown, receiver) = watch::channel(false);
        let finite = self.finite.clone();
        let stream = self.stream.clone();
        let task = tokio::spawn(observer.run_until_shutdown(finite, stream, receiver));
        (shutdown, task)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn observer() -> WorkspaceObservations {
    let installation = INSTALLATION.parse().unwrap();
    let database = Database::connect("sqlite::memory:", installation)
        .await
        .unwrap();
    database.migrate().await.unwrap();
    WorkspaceObservations::new(database, INSTALLATION.parse().unwrap())
}

fn request(uri: &Uri, requests: &Requests) -> HashMap<String, String> {
    let query: HashMap<String, String> =
        url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
            .into_owned()
            .collect();
    requests
        .lock()
        .unwrap()
        .push((uri.path().to_owned(), query.clone()));
    query
}

fn assert_scoped(requests: &Requests) {
    let requests = requests.lock().unwrap();
    assert!(requests.iter().any(|(path, _)| path.ends_with("/pods")));
    assert!(
        requests
            .iter()
            .any(|(path, _)| path.ends_with("/statefulsets"))
    );
    for (path, query) in requests.iter() {
        assert!(
            path.ends_with(&format!("/namespaces/{WORKSPACE_NAMESPACE}/pods"))
                || path.ends_with(&format!("/namespaces/{WORKSPACE_NAMESPACE}/statefulsets"))
        );
        assert_eq!(
            query.get("labelSelector").map(String::as_str),
            Some(format!("{OWNER_INSTALLATION_LABEL}={INSTALLATION}").as_str())
        );
    }
}

fn set(id: Uuid, uid: &str, rv: &str) -> Value {
    json!({
        "apiVersion": "apps/v1", "kind": "StatefulSet",
        "metadata": {"name": format!("w-{}", workspace_short_id_for(id)), "namespace": WORKSPACE_NAMESPACE,
            "uid": uid, "generation": 1, "resourceVersion": rv,
            "labels": {(OWNER_INSTALLATION_LABEL): INSTALLATION, (WORKSPACE_ID_LABEL): id.to_string()}},
        "spec": {"replicas": 1, "selector": {}, "serviceName": "workspace",
            "template": {"metadata": {"annotations": {(GENERATION): "7"}}, "spec": {"containers": []}}},
        "status": {"replicas": 1, "readyReplicas": 1, "updatedReplicas": 1, "observedGeneration": 1,
            "currentRevision": "revision-a", "updateRevision": "revision-a"}
    })
}

fn list(pods: bool, rv: &str, cursor: Option<&str>, items: Vec<Value>) -> Response {
    Json(json!({"apiVersion": if pods {"v1"} else {"apps/v1"},
        "kind": if pods {"PodList"} else {"StatefulSetList"},
        "metadata": {"resourceVersion": rv, "continue": cursor.unwrap_or("")}, "items": items}))
    .into_response()
}

fn watch_events(events: Vec<Value>) -> Response {
    let body = events
        .into_iter()
        .map(|event| format!("{event}\n"))
        .collect::<String>();
    ([("content-type", "application/json")], body).into_response()
}

fn stalled_watch() -> Response {
    Response::new(Body::from_stream(futures::stream::pending::<
        Result<axum::body::Bytes, std::io::Error>,
    >()))
}

async fn wait_for(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(8), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("observer did not reach the expected HTTP/cache milestone");
}

async fn stop(shutdown: watch::Sender<bool>, task: JoinHandle<()>) {
    shutdown.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("shutdown must cancel blocked API/database observation work")
        .unwrap();
}

#[tokio::test]
async fn paged_list_publishes_only_complete_snapshot_and_keeps_selectors_on_continuation() {
    let observer = observer().await;
    let old = Uuid::now_v7();
    let first = Uuid::now_v7();
    let second = Uuid::now_v7();
    StatefulSet::update(
        &mut observer.state.lock().unwrap(),
        serde_json::from_value(set(old, "old", "old-rv")).unwrap(),
        INSTALLATION,
        false,
    );
    let requests = Requests::default();
    let attempts = Arc::new(AtomicUsize::new(0));
    let retry_entered = Arc::new(Notify::new());
    let allow_retry = Arc::new(Notify::new());
    let router = Router::new().fallback({
        let requests = requests.clone();
        let attempts = attempts.clone();
        let retry_entered = retry_entered.clone();
        let allow_retry = allow_retry.clone();
        move |uri: Uri| {
            let (requests, attempts, retry_entered, allow_retry) =
                (requests.clone(), attempts.clone(), retry_entered.clone(), allow_retry.clone());
            async move {
                let query = request(&uri, &requests);
                if query.contains_key("watch") { return stalled_watch(); }
                if uri.path().ends_with("/pods") { return list(true, "pods-rv", None, vec![]); }
                if !query.contains_key("continue") {
                    let attempt = attempts.fetch_add(1, Ordering::SeqCst) + 1;
                    if attempt == 2 { retry_entered.notify_one(); allow_retry.notified().await; }
                    return list(false, "snapshot:opaque/+", Some("page/+ ="), vec![set(first, "first", "one")]);
                }
                assert_eq!(query["continue"], "page/+ =");
                if attempts.load(Ordering::SeqCst) == 1 {
                    return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"apiVersion":"v1", "kind":"Status",
                        "status":"Failure", "reason":"ServiceUnavailable", "message":"injected middle page failure", "code":503}))).into_response();
                }
                list(false, "snapshot:opaque/+", None, vec![set(second, "second", "two")])
            }
        }
    });
    let server = Server::start(router).await;
    let (shutdown, task) = server.run(observer.clone());
    tokio::time::timeout(Duration::from_secs(10), retry_entered.notified())
        .await
        .unwrap();
    {
        let state = observer.state.lock().unwrap();
        assert!(state.sets.contains_key(&old));
        assert!(
            !state.sets.contains_key(&first),
            "a failed later page must not publish the first page"
        );
        assert!(!state.sets.contains_key(&second));
    }
    allow_retry.notify_one();
    wait_for(|| {
        let state = observer.state.lock().unwrap();
        state.sets.contains_key(&first)
            && state.sets.contains_key(&second)
            && !state.sets.contains_key(&old)
    })
    .await;
    wait_for(|| {
        requests.lock().unwrap().iter().any(|(path, query)| {
            path.ends_with("/statefulsets")
                && query.get("resourceVersion").map(String::as_str) == Some("snapshot:opaque/+")
        })
    })
    .await;
    assert_scoped(&requests);
    let pages = requests
        .lock()
        .unwrap()
        .iter()
        .filter(|(path, query)| path.ends_with("/statefulsets") && !query.contains_key("watch"))
        .count();
    assert_eq!(pages, 4);
    stop(shutdown, task).await;
}

#[tokio::test]
async fn expired_watch_relists_all_pages_and_eof_resumes_from_last_opaque_event_version() {
    let observer = observer().await;
    let old = Uuid::now_v7();
    let first = Uuid::now_v7();
    let second = Uuid::now_v7();
    let requests = Requests::default();
    let lists = Arc::new(AtomicUsize::new(0));
    let watches = Arc::new(AtomicUsize::new(0));
    let router = Router::new().fallback({
        let requests = requests.clone();
        let lists = lists.clone();
        let watches = watches.clone();
        move |uri: Uri| {
            let (requests, lists, watches) = (requests.clone(), lists.clone(), watches.clone());
            async move {
                let query = request(&uri, &requests);
                if uri.path().ends_with("/pods") {
                    return if query.contains_key("watch") { stalled_watch() } else { list(true, "pod-rv", None, vec![]) };
                }
                if query.contains_key("watch") {
                    return match watches.fetch_add(1, Ordering::SeqCst) {
                        0 => {
                            assert_eq!(query["resourceVersion"], "initial:opaque/+1");
                            watch_events(vec![json!({"type":"MODIFIED", "object":set(old, "old", "before-410:opaque/+")}),
                                json!({"type":"ERROR", "object":{"apiVersion":"v1", "kind":"Status", "status":"Failure", "reason":"Expired", "code":410, "message":"expired"}})])
                        }
                        1 => {
                            assert_eq!(query["resourceVersion"], "relisted:opaque/+2");
                            watch_events(vec![json!({"type":"MODIFIED", "object":set(first, "modified", "last-event:opaque/+3")})])
                        }
                        _ => { assert_eq!(query["resourceVersion"], "last-event:opaque/+3"); stalled_watch() }
                    };
                }
                if !query.contains_key("continue") { lists.fetch_add(1, Ordering::SeqCst); }
                let relisted = lists.load(Ordering::SeqCst) > 1;
                let rv = if relisted { "relisted:opaque/+2" } else { "initial:opaque/+1" };
                if query.contains_key("continue") {
                    assert_eq!(query["continue"], "second/page+opaque");
                    list(false, rv, None, if relisted { vec![set(second, "second", "b")] } else { vec![] })
                } else {
                    list(false, rv, Some("second/page+opaque"), vec![if relisted { set(first, "first", "a") } else { set(old, "old", "old") }])
                }
            }
        }
    });
    let server = Server::start(router).await;
    let (shutdown, task) = server.run(observer.clone());
    wait_for(|| watches.load(Ordering::SeqCst) >= 3).await;
    {
        let state = observer.state.lock().unwrap();
        assert!(!state.sets.contains_key(&old));
        assert_eq!(state.sets[&first].uid, "modified");
        assert_eq!(state.sets[&second].uid, "second");
    }
    assert_eq!(
        lists.load(Ordering::SeqCst),
        2,
        "EOF must continue watching without another LIST"
    );
    let versions: Vec<String> = requests
        .lock()
        .unwrap()
        .iter()
        .filter(|(path, query)| path.ends_with("/statefulsets") && query.contains_key("watch"))
        .map(|(_, query)| query["resourceVersion"].clone())
        .collect();
    assert_eq!(
        versions,
        [
            "initial:opaque/+1",
            "relisted:opaque/+2",
            "last-event:opaque/+3"
        ]
    );
    let pages = requests
        .lock()
        .unwrap()
        .iter()
        .filter(|(path, query)| path.ends_with("/statefulsets") && !query.contains_key("watch"))
        .count();
    assert_eq!(
        pages, 4,
        "both initial and expired snapshots must finish both pages"
    );
    assert_scoped(&requests);
    stop(shutdown, task).await;
}

#[tokio::test]
async fn shutdown_cancels_unresponsive_list_watch_headers_body_and_database_wake() {
    for mode in ["list", "watch-headers", "watch-body", "wake"] {
        let directory = tempfile::tempdir().unwrap();
        let database_url = format!(
            "sqlite://{}",
            directory.path().join("observer.db").display()
        );
        let observer = if mode == "wake" {
            let database = Database::connect(&database_url, INSTALLATION.parse().unwrap())
                .await
                .unwrap();
            database.migrate().await.unwrap();
            WorkspaceObservations::new(database, INSTALLATION.parse().unwrap())
        } else {
            observer().await
        };
        let workspace = Uuid::now_v7();
        let job = ParkedWorkspaceJob {
            id: Uuid::now_v7(),
            workspace_id: workspace,
            payload_json: "locked-park".into(),
            until: unix_now() + 60,
            generation: 7,
            reconciled_at: unix_now(),
            desired_replicas: 1,
        };
        if mode == "wake" {
            assert!(
                observer
                    .state
                    .lock()
                    .unwrap()
                    .register(job.clone(), unix_now())
                    .is_empty()
            );
        }
        let Database::Sqlite { pool, .. } = &observer.database else {
            unreachable!()
        };
        // A separate connection holds a real SQLite write lock. WAL still lets
        // repair read; the HTTP Ready snapshot then blocks in wake's UPDATE.
        let write_lock = if mode == "wake" {
            let mut connection = sqlx::SqliteConnection::connect(&database_url)
                .await
                .unwrap();
            sqlx::query("BEGIN IMMEDIATE")
                .execute(&mut connection)
                .await
                .unwrap();
            Some(connection)
        } else {
            None
        };
        let requests = Requests::default();
        let entered = Arc::new(AtomicUsize::new(0));
        let router = Router::new().fallback({
            let requests = requests.clone();
            let entered = entered.clone();
            move |uri: Uri| {
                let (requests, entered) = (requests.clone(), entered.clone());
                async move {
                    let query = request(&uri, &requests);
                    let pods = uri.path().ends_with("/pods");
                    if mode == "list" {
                        entered.fetch_add(1, Ordering::SeqCst);
                        return std::future::pending::<Response>().await;
                    }
                    if query.contains_key("watch") {
                        entered.fetch_add(1, Ordering::SeqCst);
                        return if mode == "watch-headers" {
                            std::future::pending::<Response>().await
                        } else {
                            stalled_watch()
                        };
                    }
                    list(
                        pods,
                        "shutdown-rv",
                        None,
                        if mode == "wake" && !pods {
                            vec![set(workspace, "wake-set", "ready")]
                        } else {
                            vec![]
                        },
                    )
                }
            }
        });
        let server = Server::start(router).await;
        let (shutdown, task) = server.run(observer.clone());
        wait_for(|| {
            let requests = requests.lock().unwrap();
            requests.iter().any(|(path, _)| path.ends_with("/pods"))
                && requests
                    .iter()
                    .any(|(path, _)| path.ends_with("/statefulsets"))
        })
        .await;
        if mode == "wake" {
            wait_for(|| {
                observer
                    .state
                    .lock()
                    .unwrap()
                    .cycles
                    .get(&Cycle::for_job(&job))
                    .is_some_and(|cycle| !cycle.in_flight.is_empty())
                    && pool.num_idle() == 0
            })
            .await;
        } else {
            wait_for(|| entered.load(Ordering::SeqCst) >= 2).await;
        }
        stop(shutdown, task).await;
        if mode == "wake" {
            let state = observer.state.lock().unwrap();
            let history = &state.cycles[&Cycle::for_job(&job)];
            assert!(
                history.in_flight.is_empty(),
                "cancelling the stalled SQL wake must release its reservation"
            );
            assert!(
                history.consumed.is_empty(),
                "a cancelled wake was never delivered"
            );
        }
        if let Some(mut connection) = write_lock {
            sqlx::query("ROLLBACK")
                .execute(&mut connection)
                .await
                .unwrap();
            connection.close().await.unwrap();
        }
        assert_scoped(&requests);
    }
}
