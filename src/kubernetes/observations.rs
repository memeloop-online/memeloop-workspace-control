//! Shared, advisory runtime observations. The worker's finite GET and database
//! generation fence remain authoritative; a watch only advances a parked job.
use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use futures::{Stream, TryStreamExt};
use k8s_openapi::{
    api::{apps::v1::StatefulSet, core::v1::Pod},
    apimachinery::pkg::apis::meta::v1::ObjectMeta,
};
use kube::{
    Api, Resource, ResourceExt,
    api::{ListParams, WatchEvent, WatchParams},
};
use serde::de::DeserializeOwned;
use tokio::sync::{Notify, watch};
use uuid::Uuid;

use super::{OWNER_INSTALLATION_LABEL, WORKSPACE_ID_LABEL};
use crate::{
    config::InstallationId,
    storage::{Database, ParkedWorkspaceJob, StorageError},
    workspace_runtime::{WORKSPACE_NAMESPACE, workspace_short_id_for},
};

const GENERATION: &str = "workspace.memeloop.dev/generation";
const REPAIR_SECONDS: u64 = 30;

#[derive(Clone)]
pub struct WorkspaceObservations {
    database: Database,
    installation_id: InstallationId,
    state: Arc<Mutex<State>>,
    job_wakeup: Arc<Notify>,
}

#[derive(Default)]
struct State {
    pods: HashMap<Uuid, PodView>,
    sets: HashMap<Uuid, SetView>,
    waiters: HashMap<Uuid, ParkedWorkspaceJob>,
    registration_ticket: u64,
    waiter_tickets: HashMap<Uuid, u64>,
    by_workspace: HashMap<Uuid, HashSet<Uuid>>,
    cycles: HashMap<Cycle, CycleHistory>,
    edges: HashMap<Uuid, HashMap<(u64, i32), Edge>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Cycle {
    job_id: Uuid,
    generation: u64,
    reconciled_at: i64,
}

impl Cycle {
    fn for_job(job: &ParkedWorkspaceJob) -> Self {
        Self {
            job_id: job.id,
            generation: job.generation,
            reconciled_at: job.reconciled_at,
        }
    }
}

#[derive(Default)]
struct CycleHistory {
    until: i64,
    workspace_id: Uuid,
    desired_replicas: i32,
    consumed: HashSet<u64>,
    // A failed CAS invalidates that park, not the edge for its successor nonce.
    attempted: HashSet<(String, u64)>,
    in_flight: HashMap<u64, String>,
    retry_after: HashMap<u64, i64>,
}

#[derive(Default)]
struct Edge {
    candidate: Option<Candidate>,
    epoch: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Candidate {
    set_uid: String,
    revision: Option<String>,
    generation: u64,
    pod_uid: Option<String>,
}

#[derive(Clone)]
struct Reservation {
    job: ParkedWorkspaceJob,
    cycle: Cycle,
    edge: u64,
}

/// A cancelled hook/stream must release every reservation submitted to SQL.
struct WakeBatch {
    state: Arc<Mutex<State>>,
    reservations: Vec<Reservation>,
}

impl Drop for WakeBatch {
    fn drop(&mut self) {
        if self.reservations.is_empty() {
            return;
        }
        let mut state = self
            .state
            .lock()
            .expect("workspace observations lock poisoned");
        let now = unix_now();
        for reservation in &self.reservations {
            state.release_cancelled(reservation, now);
        }
    }
}

#[derive(Clone)]
struct SetView {
    uid: String,
    generation: u64,
    desired: i32,
    observed: bool,
    terminating: bool,
    replicas: i32,
    ready: i32,
    updated: i32,
    current_revision: Option<String>,
    update_revision: Option<String>,
}

#[derive(Clone)]
struct PodView {
    uid: String,
    generation: u64,
    owner_uid: String,
    revision: Option<String>,
    ready: bool,
}

impl WorkspaceObservations {
    pub fn new(database: Database, installation_id: InstallationId) -> Self {
        Self {
            database,
            installation_id,
            state: Arc::new(Mutex::new(State::default())),
            job_wakeup: Arc::new(Notify::new()),
        }
    }

    pub fn with_job_wakeup(mut self, job_wakeup: Arc<Notify>) -> Self {
        self.job_wakeup = job_wakeup;
        self
    }

    /// Called after the atomic park and workspace-lease release. Fetch the exact
    /// persisted nonce, then inspect the current cache under the registration lock.
    pub async fn register(&self, job_id: Uuid, now: i64) -> Result<(), StorageError> {
        let ticket = self
            .state
            .lock()
            .expect("workspace observations lock poisoned")
            .next_ticket();
        let Some(job) = self.database.get_parked_workspace_job(job_id, now).await? else {
            return Ok(());
        };
        let reservations = {
            let mut state = self
                .state
                .lock()
                .expect("workspace observations lock poisoned");
            state.register_at(job, now, ticket)
        };
        self.deliver(reservations).await;
        Ok(())
    }

    pub async fn run_until_shutdown(
        self,
        finite_client: kube::Client,
        stream_client: kube::Client,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let observe_shutdown = shutdown.clone();
        let observe = async {
            tokio::join!(
                self.watch_resource::<Pod>(
                    finite_client.clone(),
                    stream_client.clone(),
                    observe_shutdown.clone()
                ),
                self.watch_resource::<StatefulSet>(
                    finite_client,
                    stream_client,
                    observe_shutdown.clone()
                ),
                self.repair_loop(observe_shutdown.clone()),
            );
        };
        tokio::select! {
            _ = shutdown.changed() => {},
            _ = observe => {},
        }
    }

    async fn repair_loop(&self, mut shutdown: watch::Receiver<bool>) {
        let mut interval = tokio::time::interval(Duration::from_secs(REPAIR_SECONDS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                _ = shutdown.changed() => break,
                _ = interval.tick() => {
                    if *shutdown.borrow() { break; }
                    let repair = self.repair();
                    tokio::pin!(repair);
                    tokio::select! {
                        _ = shutdown.changed() => break,
                        result = &mut repair => if let Err(error) = result {
                            tracing::warn!(%error, "workspace observation repair failed");
                        },
                    }
                }
            }
        }
    }

    async fn repair(&self) -> Result<(), StorageError> {
        let now = unix_now();
        let mut after = None;
        loop {
            let ticket = self
                .state
                .lock()
                .expect("workspace observations lock poisoned")
                .next_ticket();
            let page = self
                .database
                .list_parked_workspace_jobs(after, 256, now)
                .await?;
            let reservations = {
                let mut state = self
                    .state
                    .lock()
                    .expect("workspace observations lock poisoned");
                let mut reservations = Vec::new();
                for job in page.jobs {
                    reservations.extend(state.register_at(job, now, ticket));
                }
                reservations
            };
            self.deliver(reservations).await;
            match page.next {
                Some(cursor) => after = Some(cursor),
                None => break,
            }
        }
        // Do not remove newly registered jobs absent from an earlier scan page.
        // Expiry is sufficient to bound active waiters; cycle history must survive
        // a running job and its next park until the original deadline.
        let mut state = self
            .state
            .lock()
            .expect("workspace observations lock poisoned");
        state.prune(now);
        Ok(())
    }

    async fn deliver(&self, reservations: Vec<Reservation>) {
        if tokio::time::timeout(Duration::from_secs(5), self.deliver_batch(reservations))
            .await
            .is_err()
        {
            tracing::warn!("workspace observation wake batch timed out");
        }
    }

    async fn deliver_batch(&self, reservations: Vec<Reservation>) {
        let mut batch = WakeBatch {
            state: self.state.clone(),
            reservations,
        };
        while !batch.reservations.is_empty() {
            let jobs: Vec<_> = batch
                .reservations
                .iter()
                .map(|reservation| reservation.job.clone())
                .collect();
            let started = Instant::now();
            let result = self
                .database
                .wake_parked_workspace_jobs(&jobs, unix_now())
                .await;
            if let Err(error) = &result {
                tracing::warn!(%error, count = jobs.len(), "workspace observation wake failed");
            }
            if let Ok(awakened) = &result {
                if !awakened.is_empty() {
                    self.job_wakeup.notify_waiters();
                }
                for reservation in &batch.reservations {
                    if awakened.contains(&reservation.job.id) {
                        tracing::info!(job_id = %reservation.job.id,
                            workspace_id = %reservation.job.workspace_id,
                            generation = reservation.job.generation,
                            elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
                            batch_size = jobs.len(),
                            "workspace readiness job awakened");
                    }
                }
            }
            let mut next = Vec::new();
            {
                let mut state = self
                    .state
                    .lock()
                    .expect("workspace observations lock poisoned");
                let now = unix_now();
                for reservation in &batch.reservations {
                    let outcome = result
                        .as_ref()
                        .map(|awakened| awakened.contains(&reservation.job.id))
                        .map_err(|_| ());
                    next.extend(state.complete(reservation, outcome, now));
                }
            }
            batch.reservations = next;
        }
    }

    async fn watch_resource<K: ObservedResource>(
        &self,
        finite_client: kube::Client,
        stream_client: kube::Client,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let selector = format!(
            "{OWNER_INSTALLATION_LABEL}={}",
            self.installation_id.as_str()
        );
        let finite_api = Api::<K>::namespaced(finite_client, WORKSPACE_NAMESPACE);
        let stream_api = Api::<K>::namespaced(stream_client, WORKSPACE_NAMESPACE);
        let params = WatchParams::default().labels(&selector).timeout(60);
        let list_params = ListParams::default().labels(&selector).limit(256);
        let mut resource_version = None;
        loop {
            if *shutdown.borrow() {
                return;
            }
            if resource_version.is_none() {
                let result = tokio::select! {
                    _ = shutdown.changed() => return,
                    result = self.publish_snapshot(&finite_api, &list_params) => result,
                };
                match result {
                    Ok(version) => resource_version = version,
                    Err(error) => {
                        tracing::warn!(%error, resource = %K::kind(&()), "workspace observation list failed");
                        if wait_retry(&mut shutdown).await {
                            return;
                        }
                        continue;
                    }
                }
            }
            if resource_version.is_none() {
                if wait_retry(&mut shutdown).await {
                    return;
                }
                continue;
            }
            match self
                .watch_once(&stream_api, &params, &mut resource_version, &mut shutdown)
                .await
            {
                WatchOutcome::Shutdown => return,
                WatchOutcome::Relist => resource_version = None,
                WatchOutcome::Retry if resource_version.is_some() => {
                    if wait_retry(&mut shutdown).await {
                        return;
                    }
                }
                WatchOutcome::Resume | WatchOutcome::Retry => {}
            }
        }
    }

    async fn publish_snapshot<K: ObservedResource>(
        &self,
        api: &Api<K>,
        params: &ListParams,
    ) -> Result<Option<String>, kube::Error> {
        let (snapshot, version) = list_snapshot(api, params, self.installation_id.as_str()).await?;
        let reservations = {
            let mut state = self
                .state
                .lock()
                .expect("workspace observations lock poisoned");
            K::replace(&mut state, snapshot);
            state.refresh_all(unix_now())
        };
        self.deliver(reservations).await;
        Ok(version)
    }

    async fn watch_once<K: ObservedResource>(
        &self,
        api: &Api<K>,
        params: &WatchParams,
        resource_version: &mut Option<String>,
        shutdown: &mut watch::Receiver<bool>,
    ) -> WatchOutcome {
        let connected = tokio::select! {
            _ = shutdown.changed() => return WatchOutcome::Shutdown,
            result = tokio::time::timeout(Duration::from_secs(20), api.watch(params, resource_version.as_deref().expect("listed resource version"))) => result,
        };
        match connected {
            Ok(Ok(stream)) => {
                self.consume_stream(stream, resource_version, shutdown)
                    .await
            }
            Ok(Err(kube::Error::Api(error))) if error.code == 410 => WatchOutcome::Relist,
            Ok(Err(error)) => {
                tracing::warn!(%error, resource = %K::kind(&()), "workspace observation watch connect failed");
                WatchOutcome::Retry
            }
            Err(error) => {
                tracing::warn!(%error, resource = %K::kind(&()), "workspace observation watch connect timed out");
                WatchOutcome::Retry
            }
        }
    }

    async fn consume_stream<K: ObservedResource>(
        &self,
        stream: impl Stream<Item = Result<WatchEvent<K>, kube::Error>>,
        resource_version: &mut Option<String>,
        shutdown: &mut watch::Receiver<bool>,
    ) -> WatchOutcome {
        tokio::pin!(stream);
        let started = tokio::time::Instant::now();
        loop {
            let event = tokio::select! {
                _ = shutdown.changed() => return WatchOutcome::Shutdown,
                result = tokio::time::timeout(Duration::from_secs(75), stream.try_next()) => result,
            };
            match event {
                Ok(Ok(Some(event))) => {
                    if let Some(outcome) = self.apply_event(event, resource_version).await {
                        return outcome;
                    }
                }
                Ok(Ok(None)) => {
                    // Normal server expiry resumes immediately; unexpectedly short
                    // streams back off to avoid reconnect loops.
                    return if started.elapsed() < Duration::from_secs(1) {
                        WatchOutcome::Retry
                    } else {
                        WatchOutcome::Resume
                    };
                }
                Ok(Err(error)) => {
                    tracing::warn!(%error, resource = %K::kind(&()), "workspace observation stream disconnected");
                    return if matches!(&error, kube::Error::Api(response) if response.code == 410) {
                        WatchOutcome::Relist
                    } else {
                        WatchOutcome::Retry
                    };
                }
                Err(_) => {
                    tracing::warn!(resource = %K::kind(&()), "workspace observation stream idle timeout");
                    return WatchOutcome::Retry;
                }
            }
        }
    }

    async fn apply_event<K: ObservedResource>(
        &self,
        event: WatchEvent<K>,
        resource_version: &mut Option<String>,
    ) -> Option<WatchOutcome> {
        let (object, deleted) = match event {
            WatchEvent::Added(object) | WatchEvent::Modified(object) => (object, false),
            WatchEvent::Deleted(object) => (object, true),
            WatchEvent::Bookmark(bookmark) => {
                *resource_version = Some(bookmark.metadata.resource_version);
                return None;
            }
            WatchEvent::Error(error) => {
                tracing::warn!(%error, resource = %K::kind(&()), "workspace observation watch error");
                return Some(if error.code == 410 {
                    WatchOutcome::Relist
                } else {
                    WatchOutcome::Retry
                });
            }
        };
        *resource_version = object.resource_version();
        let reservations = {
            let mut state = self
                .state
                .lock()
                .expect("workspace observations lock poisoned");
            let workspace =
                owned_workspace(object.meta(), self.installation_id.as_str(), K::IS_POD);
            K::update(&mut state, object, self.installation_id.as_str(), deleted);
            workspace
                .map(|id| state.refresh_workspace(id, unix_now()))
                .unwrap_or_default()
        };
        self.deliver(reservations).await;
        None
    }
}

impl State {
    fn next_ticket(&mut self) -> u64 {
        self.registration_ticket += 1;
        self.registration_ticket
    }

    #[cfg(test)]
    fn register(&mut self, job: ParkedWorkspaceJob, now: i64) -> Vec<Reservation> {
        let ticket = self.next_ticket();
        self.register_at(job, now, ticket)
    }

    fn register_at(&mut self, job: ParkedWorkspaceJob, now: i64, ticket: u64) -> Vec<Reservation> {
        if job.until <= now
            || self
                .waiter_tickets
                .get(&job.id)
                .is_some_and(|current| *current > ticket)
        {
            return Vec::new();
        }
        self.waiter_tickets.insert(job.id, ticket);
        let history = self.cycles.entry(Cycle::for_job(&job)).or_default();
        history.until = job.until;
        history.workspace_id = job.workspace_id;
        history.desired_replicas = job.desired_replicas;
        if let Some(previous) = self.waiters.insert(job.id, job.clone())
            && let Some(jobs) = self.by_workspace.get_mut(&previous.workspace_id)
        {
            jobs.remove(&previous.id);
        }
        self.by_workspace
            .entry(job.workspace_id)
            .or_default()
            .insert(job.id);
        self.refresh_workspace(job.workspace_id, now)
    }

    fn refresh_all(&mut self, now: i64) -> Vec<Reservation> {
        let workspaces: Vec<_> = self
            .edges
            .keys()
            .chain(self.by_workspace.keys())
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        workspaces
            .into_iter()
            .flat_map(|id| self.refresh_workspace(id, now))
            .collect()
    }

    fn refresh_workspace(&mut self, workspace_id: Uuid, now: i64) -> Vec<Reservation> {
        let jobs: Vec<_> = self
            .by_workspace
            .get(&workspace_id)
            .into_iter()
            .flatten()
            .filter_map(|id| self.waiters.get(id).cloned())
            .collect();
        let keys: HashSet<_> = self
            .edges
            .get(&workspace_id)
            .into_iter()
            .flat_map(|edges| edges.keys().copied())
            .chain(
                jobs.iter()
                    .map(|job| (job.generation, job.desired_replicas)),
            )
            .collect();
        for (generation, desired) in keys {
            let candidate = self.candidate(workspace_id, generation, desired);
            let edge = self
                .edges
                .entry(workspace_id)
                .or_default()
                .entry((generation, desired))
                .or_default();
            if edge.candidate != candidate {
                if candidate.is_some() {
                    edge.epoch += 1;
                }
                edge.candidate = candidate;
            }
        }
        jobs.into_iter()
            .filter_map(|job| self.reserve(job, now))
            .collect()
    }

    fn candidate(&self, workspace_id: Uuid, generation: u64, desired: i32) -> Option<Candidate> {
        let set = self.sets.get(&workspace_id)?;
        if set.generation != generation
            || set.desired != desired
            || !set.observed
            || set.terminating
        {
            return None;
        }
        let base = Candidate {
            set_uid: set.uid.clone(),
            revision: set.update_revision.clone(),
            generation,
            pod_uid: None,
        };
        if desired == 0 {
            return (set.replicas == 0).then_some(base);
        }
        if desired != 1 {
            return None;
        }
        if let Some(pod) = self.pods.get(&workspace_id).filter(|pod| {
            pod.generation == generation
                && pod.owner_uid == set.uid
                && set.update_revision.is_some()
                && pod.revision == set.update_revision
        }) {
            // Keep the same identity while StatefulSet aggregates catch up. An
            // observed current NotReady Pod also overrides a stale aggregate.
            return pod.ready.then_some(Candidate {
                pod_uid: Some(pod.uid.clone()),
                ..base
            });
        }
        (set.replicas == 1
            && set.ready == 1
            && set.updated == 1
            && set.current_revision.is_some()
            && set.current_revision == set.update_revision)
            .then_some(base)
    }

    fn reserve(&mut self, job: ParkedWorkspaceJob, now: i64) -> Option<Reservation> {
        if job.until <= now {
            return None;
        }
        let edge = self
            .edges
            .get(&job.workspace_id)?
            .get(&(job.generation, job.desired_replicas))?;
        edge.candidate.as_ref()?;
        let epoch = edge.epoch;
        let cycle = Cycle::for_job(&job);
        let history = self.cycles.get_mut(&cycle)?;
        if history.consumed.contains(&epoch)
            || history.in_flight.contains_key(&epoch)
            || history
                .attempted
                .contains(&(job.payload_json.clone(), epoch))
            || history
                .retry_after
                .get(&epoch)
                .is_some_and(|deadline| *deadline > now)
        {
            return None;
        }
        history.in_flight.insert(epoch, job.payload_json.clone());
        Some(Reservation {
            job,
            cycle,
            edge: epoch,
        })
    }

    fn complete(
        &mut self,
        reservation: &Reservation,
        result: Result<bool, ()>,
        now: i64,
    ) -> Vec<Reservation> {
        let Some(history) = self.cycles.get_mut(&reservation.cycle) else {
            return Vec::new();
        };
        if history.in_flight.get(&reservation.edge) != Some(&reservation.job.payload_json) {
            return Vec::new();
        }
        history.in_flight.remove(&reservation.edge);
        match result {
            Ok(true) => {
                history.consumed.insert(reservation.edge);
            }
            Ok(false) => {
                history
                    .attempted
                    .insert((reservation.job.payload_json.clone(), reservation.edge));
            }
            Err(()) => {
                history
                    .retry_after
                    .insert(reservation.edge, now.saturating_add(REPAIR_SECONDS as i64));
            }
        }
        // A new nonce may have registered while the old wake was in flight. It
        // gets its own attempt after a zero-row result; never overwrite its waiter.
        self.waiters
            .get(&reservation.job.id)
            .cloned()
            .and_then(|job| self.reserve(job, now))
            .into_iter()
            .collect()
    }

    fn release_cancelled(&mut self, reservation: &Reservation, now: i64) {
        if let Some(history) = self.cycles.get_mut(&reservation.cycle)
            && history.in_flight.get(&reservation.edge) == Some(&reservation.job.payload_json)
        {
            history.in_flight.remove(&reservation.edge);
            history
                .retry_after
                .insert(reservation.edge, now.saturating_add(REPAIR_SECONDS as i64));
        }
    }

    fn prune(&mut self, now: i64) {
        self.waiters.retain(|_, job| job.until > now);
        self.waiter_tickets
            .retain(|id, _| self.waiters.contains_key(id));
        self.by_workspace.retain(|_, ids| {
            ids.retain(|id| self.waiters.contains_key(id));
            !ids.is_empty()
        });
        self.cycles.retain(|_, history| history.until > now);
        let live_scopes: HashSet<_> = self
            .cycles
            .iter()
            .map(|(cycle, history)| {
                (
                    history.workspace_id,
                    cycle.generation,
                    history.desired_replicas,
                )
            })
            .collect();
        self.edges.retain(|workspace_id, edges| {
            edges.retain(|(generation, desired), _| {
                live_scopes.contains(&(*workspace_id, *generation, *desired))
            });
            !edges.is_empty()
        });
    }
}

enum WatchOutcome {
    Resume,
    Relist,
    Retry,
    Shutdown,
}

trait ObservedResource:
    Resource<DynamicType = (), Scope = kube::core::NamespaceResourceScope>
    + Clone
    + Debug
    + DeserializeOwned
    + Send
    + Sync
    + 'static
{
    const IS_POD: bool;
    fn replace(state: &mut State, snapshot: State);
    fn update(state: &mut State, object: Self, installation: &str, deleted: bool)
    where
        Self: Sized;
}

/// Build only minimal views while following the API's consistent list snapshot.
/// Nothing is published until the final page; a failed/expired continuation is
/// discarded and the next attempt starts again from the first page.
async fn list_snapshot<K>(
    api: &Api<K>,
    params: &ListParams,
    installation: &str,
) -> Result<(State, Option<String>), kube::Error>
where
    K: ObservedResource + Clone + Debug + DeserializeOwned,
{
    let mut snapshot = State::default();
    let mut params = params.clone();
    let mut version = None;
    loop {
        let page = api.list(&params).await?;
        if version.is_none() {
            version = page.metadata.resource_version;
        }
        for object in page.items {
            K::update(&mut snapshot, object, installation, false);
        }
        match page.metadata.continue_.filter(|cursor| !cursor.is_empty()) {
            Some(cursor) => params = params.continue_token(&cursor),
            None => return Ok((snapshot, version)),
        }
    }
}

fn owned_workspace(metadata: &ObjectMeta, installation: &str, pod: bool) -> Option<Uuid> {
    if metadata.namespace.as_deref() != Some(WORKSPACE_NAMESPACE) {
        return None;
    }
    let labels = metadata.labels.as_ref()?;
    if labels.get(OWNER_INSTALLATION_LABEL).map(String::as_str) != Some(installation) {
        return None;
    }
    let id = Uuid::parse_str(labels.get(WORKSPACE_ID_LABEL)?).ok()?;
    let prefix = format!("w-{}", workspace_short_id_for(id));
    let expected = if pod { format!("{prefix}-0") } else { prefix };
    (metadata.name.as_deref() == Some(expected.as_str())).then_some(id)
}

impl ObservedResource for StatefulSet {
    const IS_POD: bool = false;
    fn replace(state: &mut State, snapshot: State) {
        state.sets = snapshot.sets;
    }

    fn update(state: &mut State, object: Self, installation: &str, deleted: bool) {
        let Some(id) = owned_workspace(&object.metadata, installation, false) else {
            return;
        };
        if deleted {
            if state
                .sets
                .get(&id)
                .is_some_and(|cached| Some(cached.uid.as_str()) == object.metadata.uid.as_deref())
            {
                state.sets.remove(&id);
            }
            return;
        }
        let Some(uid) = object.metadata.uid else {
            return;
        };
        let Some(spec) = object.spec else {
            return;
        };
        let Some(status) = object.status else {
            state.sets.remove(&id);
            return;
        };
        let Some(generation) = spec
            .template
            .metadata
            .and_then(|m| m.annotations)
            .and_then(|a| a.get(GENERATION).and_then(|v| v.parse().ok()))
        else {
            state.sets.remove(&id);
            return;
        };
        state.sets.insert(
            id,
            SetView {
                uid,
                generation,
                desired: spec.replicas.unwrap_or(-1),
                observed: object.metadata.generation.is_some()
                    && status.observed_generation.is_some()
                    && status.observed_generation >= object.metadata.generation,
                terminating: object.metadata.deletion_timestamp.is_some(),
                replicas: status.replicas,
                ready: status.ready_replicas.unwrap_or_default(),
                updated: status.updated_replicas.unwrap_or_default(),
                current_revision: status.current_revision,
                update_revision: status.update_revision,
            },
        );
    }
}

impl ObservedResource for Pod {
    const IS_POD: bool = true;
    fn replace(state: &mut State, snapshot: State) {
        state.pods = snapshot.pods;
    }

    fn update(state: &mut State, object: Self, installation: &str, deleted: bool) {
        let Some(id) = owned_workspace(&object.metadata, installation, true) else {
            return;
        };
        if deleted {
            if state
                .pods
                .get(&id)
                .is_some_and(|cached| Some(cached.uid.as_str()) == object.metadata.uid.as_deref())
            {
                state.pods.remove(&id);
            }
            return;
        }
        let Some(uid) = object.metadata.uid else {
            return;
        };
        let Some(generation) = object
            .metadata
            .annotations
            .as_ref()
            .and_then(|a| a.get(GENERATION))
            .and_then(|v| v.parse().ok())
        else {
            state.pods.remove(&id);
            return;
        };
        let expected_owner = format!("w-{}", workspace_short_id_for(id));
        let Some(owner_uid) = object
            .metadata
            .owner_references
            .as_ref()
            .and_then(|owners| {
                owners.iter().find(|owner| {
                    owner.controller == Some(true)
                        && owner.api_version == "apps/v1"
                        && owner.kind == "StatefulSet"
                        && owner.name == expected_owner
                })
            })
            .map(|owner| owner.uid.clone())
        else {
            state.pods.remove(&id);
            return;
        };
        state.pods.insert(
            id,
            PodView {
                uid,
                generation,
                owner_uid,
                revision: object
                    .metadata
                    .labels
                    .as_ref()
                    .and_then(|labels| labels.get("controller-revision-hash"))
                    .cloned(),
                ready: object.metadata.deletion_timestamp.is_none()
                    && object.status.is_some_and(|status| {
                        status.phase.as_deref() == Some("Running")
                            && status.conditions.is_some_and(|conditions| {
                                conditions.iter().any(|condition| {
                                    condition.type_ == "Ready" && condition.status == "True"
                                })
                            })
                    }),
            },
        );
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_secs() as i64
}

async fn wait_retry(shutdown: &mut watch::Receiver<bool>) -> bool {
    tokio::select! {
        _ = shutdown.changed() => true,
        _ = tokio::time::sleep(Duration::from_secs(5)) => *shutdown.borrow(),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod http_tests;
