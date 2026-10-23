use super::*;
use serde_json::json;

fn job() -> ParkedWorkspaceJob {
    ParkedWorkspaceJob {
        id: Uuid::now_v7(),
        workspace_id: Uuid::now_v7(),
        payload_json: "park-a".into(),
        until: 160,
        generation: 7,
        reconciled_at: 100,
        desired_replicas: 1,
    }
}

fn ready_set() -> SetView {
    SetView {
        uid: "set-a".into(),
        generation: 7,
        desired: 1,
        observed: true,
        terminating: false,
        replicas: 1,
        ready: 1,
        updated: 1,
        current_revision: Some("revision-a".into()),
        update_revision: Some("revision-a".into()),
    }
}

fn ready_state(job: &ParkedWorkspaceJob) -> State {
    let mut state = State::default();
    state.sets.insert(job.workspace_id, ready_set());
    state
}

#[test]
fn ready_before_registration_wakes_and_same_edge_survives_repark_repair() {
    let mut job = job();
    let mut state = ready_state(&job);
    let wake = state.register(job.clone(), 101).pop().unwrap();
    assert!(state.complete(&wake, Ok(true), 101).is_empty());
    job.payload_json = "park-b".into();
    assert!(state.register(job.clone(), 102).is_empty());
    assert!(state.refresh_all(130).is_empty());
    // An ordinary watch update or same-identity 410 relist is not a new edge.
    state.sets.insert(job.workspace_id, ready_set());
    assert!(state.refresh_workspace(job.workspace_id, 131).is_empty());
    // A genuine false->true transition is a new edge in the same cycle.
    state.sets.get_mut(&job.workspace_id).unwrap().ready = 0;
    assert!(state.refresh_workspace(job.workspace_id, 132).is_empty());
    state.sets.get_mut(&job.workspace_id).unwrap().ready = 1;
    assert_eq!(state.refresh_workspace(job.workspace_id, 133).len(), 1);
}

#[test]
fn zero_rows_while_running_does_not_consume_ready_for_the_next_nonce() {
    let mut job = job();
    let mut state = ready_state(&job);
    let old = state.register(job.clone(), 101).pop().unwrap();
    assert!(state.complete(&old, Ok(false), 102).is_empty());
    // Repeated old snapshots do not continually hit SQL.
    assert!(state.refresh_all(102).is_empty());
    job.payload_json = "park-b".into();
    let next = state.register(job, 103);
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].job.payload_json, "park-b");
}

#[test]
fn delayed_zero_row_completion_rechecks_new_nonce_without_overwriting_it() {
    let mut job = job();
    let mut state = ready_state(&job);
    let old = state.register(job.clone(), 101).pop().unwrap();
    job.payload_json = "park-b".into();
    assert!(state.register(job.clone(), 102).is_empty());
    let next = state.complete(&old, Ok(false), 103);
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].job.payload_json, "park-b");
    assert_eq!(state.waiters[&job.id].payload_json, "park-b");
    assert!(state.complete(&old, Ok(true), 104).is_empty());
    assert_eq!(
        state.cycles[&Cycle::for_job(&job)].in_flight[&next[0].edge],
        "park-b"
    );
}

#[test]
fn successful_old_nonce_completion_deduplicates_the_new_nonce() {
    let mut job = job();
    let mut state = ready_state(&job);
    let old = state.register(job.clone(), 101).pop().unwrap();
    job.payload_json = "park-b".into();
    assert!(state.register(job, 102).is_empty());
    assert!(state.complete(&old, Ok(true), 103).is_empty());
    assert!(state.refresh_all(130).is_empty());
}

#[test]
fn sql_failure_retries_on_repair_cadence_without_post_hook_hot_loop() {
    let mut job = job();
    let mut state = ready_state(&job);
    let wake = state.register(job.clone(), 101).pop().unwrap();
    assert!(state.complete(&wake, Err(()), 102).is_empty());
    job.payload_json = "park-b".into();
    assert!(state.register(job, 103).is_empty());
    assert!(state.refresh_all(131).is_empty());
    assert_eq!(state.refresh_all(132).len(), 1);
}

#[test]
fn late_scan_and_late_pk_read_cannot_replace_a_newer_registration() {
    let old = job();
    let mut state = ready_state(&old);
    let scan_ticket = state.next_ticket();
    let mut new = old.clone();
    new.payload_json = "new-nonce".into();
    let new_ticket = state.next_ticket();
    assert_eq!(state.register_at(new.clone(), 101, new_ticket).len(), 1);
    assert!(state.register_at(old, 102, scan_ticket).is_empty());
    assert_eq!(state.waiters[&new.id].payload_json, "new-nonce");
}

#[test]
fn unchanged_ready_bool_with_replaced_uid_or_revision_is_a_new_edge() {
    let job = job();
    let mut state = ready_state(&job);
    let old = state.register(job.clone(), 101).pop().unwrap();
    state.complete(&old, Ok(true), 101);
    state.sets.get_mut(&job.workspace_id).unwrap().uid = "replacement-set".into();
    let replacement = state
        .refresh_workspace(job.workspace_id, 102)
        .pop()
        .unwrap();
    assert_ne!(old.edge, replacement.edge);
    state.complete(&replacement, Ok(true), 102);
    let set = state.sets.get_mut(&job.workspace_id).unwrap();
    set.current_revision = Some("revision-b".into());
    set.update_revision = Some("revision-b".into());
    assert_eq!(state.refresh_workspace(job.workspace_id, 103).len(), 1);
}

#[test]
fn anchored_deadline_disables_wake_and_reclaims_cycle_history() {
    let job = job();
    let mut state = ready_state(&job);
    state.register(job.clone(), 101);
    assert!(state.refresh_all(160).is_empty());
    state.prune(160);
    assert!(state.waiters.is_empty());
    assert!(state.cycles.is_empty());
    assert!(state.edges.is_empty());
}

#[test]
fn cache_absence_and_unobserved_or_wrong_generation_do_not_mean_stopped() {
    let mut job = job();
    job.desired_replicas = 0;
    let mut state = State::default();
    assert!(state.register(job.clone(), 101).is_empty());
    let mut set = ready_set();
    set.desired = 0;
    set.replicas = 0;
    set.observed = false;
    state.sets.insert(job.workspace_id, set);
    assert!(state.refresh_all(102).is_empty());
    let set = state.sets.get_mut(&job.workspace_id).unwrap();
    set.observed = true;
    set.generation = 6;
    assert!(state.refresh_all(103).is_empty());
    state.sets.get_mut(&job.workspace_id).unwrap().generation = 7;
    assert_eq!(state.refresh_all(104).len(), 1);
}

#[test]
fn pod_ready_fallback_requires_current_controller_revision_and_generation() {
    let job = job();
    let mut state = ready_state(&job);
    state.sets.get_mut(&job.workspace_id).unwrap().ready = 0;
    state.pods.insert(
        job.workspace_id,
        PodView {
            uid: "pod-a".into(),
            generation: 7,
            owner_uid: "old-set".into(),
            revision: Some("revision-a".into()),
            ready: true,
        },
    );
    assert!(state.register(job.clone(), 101).is_empty());
    state.pods.get_mut(&job.workspace_id).unwrap().owner_uid = "set-a".into();
    state.pods.get_mut(&job.workspace_id).unwrap().revision = Some("old-revision".into());
    assert!(state.refresh_all(102).is_empty());
    state.pods.get_mut(&job.workspace_id).unwrap().revision = Some("revision-a".into());
    state.pods.get_mut(&job.workspace_id).unwrap().generation = 6;
    assert!(state.refresh_all(103).is_empty());
    state.pods.get_mut(&job.workspace_id).unwrap().generation = 7;
    assert_eq!(state.refresh_all(104).len(), 1);
}

#[test]
fn cache_rejects_other_installation_namespace_and_noncanonical_object_names() {
    let job = job();
    let name = format!("w-{}", workspace_short_id_for(job.workspace_id));
    let mut set: StatefulSet = serde_json::from_value(json!({
        "metadata": { "name": name, "namespace": WORKSPACE_NAMESPACE, "uid": "set-a", "generation": 1,
            "labels": { (OWNER_INSTALLATION_LABEL): "test", (WORKSPACE_ID_LABEL): job.workspace_id.to_string() } },
        "spec": { "replicas": 1, "selector": {}, "serviceName": "workspace", "template": {
            "metadata": { "annotations": { (GENERATION): "7" } }, "spec": { "containers": [] } } },
        "status": { "replicas": 1, "readyReplicas": 1, "updatedReplicas": 1, "observedGeneration": 1,
            "currentRevision": "revision-a", "updateRevision": "revision-a" }
    })).unwrap();
    let mut state = State::default();
    StatefulSet::update(&mut state, set.clone(), "other-installation", false);
    assert!(state.sets.is_empty());
    set.metadata.namespace = Some("other-namespace".into());
    StatefulSet::update(&mut state, set.clone(), "test", false);
    assert!(state.sets.is_empty());
    set.metadata.namespace = Some(WORKSPACE_NAMESPACE.into());
    set.metadata.name = Some("another-workspace".into());
    StatefulSet::update(&mut state, set.clone(), "test", false);
    assert!(state.sets.is_empty());
    set.metadata.name = Some(name);
    StatefulSet::update(&mut state, set, "test", false);
    assert_eq!(state.register(job, 101).len(), 1);
}

#[test]
fn thousands_of_parked_workspaces_only_reserve_their_own_ready_events() {
    for count in [0, 100, 1000, 3000] {
        let mut state = State::default();
        let jobs: Vec<_> = (0..count).map(|_| job()).collect();
        for job in &jobs {
            assert!(state.register(job.clone(), 101).is_empty());
        }
        assert!(state.refresh_all(130).is_empty());
        for job in &jobs {
            state.sets.insert(job.workspace_id, ready_set());
            let wake = state.refresh_workspace(job.workspace_id, 131);
            assert_eq!(wake.len(), 1);
            assert_eq!(wake[0].job.id, job.id);
            state.complete(&wake[0], Ok(true), 131);
        }
        assert!(state.refresh_all(132).is_empty());
        assert_eq!(
            state
                .cycles
                .values()
                .map(|history| history.consumed.len())
                .sum::<usize>(),
            count
        );
    }
}

#[test]
fn cancelled_hook_releases_only_its_own_reservation_for_repair() {
    let job = job();
    let mut state = ready_state(&job);
    let wake = state.register(job.clone(), 101).pop().unwrap();
    state.release_cancelled(&wake, 102);
    assert!(state.cycles[&Cycle::for_job(&job)].in_flight.is_empty());
    assert!(state.refresh_all(131).is_empty());
    let replacement = state.refresh_all(132).pop().unwrap();
    let mut stale = replacement.clone();
    stale.job.payload_json = "obsolete-nonce".into();
    state.release_cancelled(&stale, 133);
    assert_eq!(
        state.cycles[&Cycle::for_job(&job)].in_flight[&replacement.edge],
        replacement.job.payload_json
    );
}

#[test]
fn repeated_not_ready_metadata_updates_have_no_wake_and_ready_is_local() {
    let a = job();
    let b = job();
    let mut state = ready_state(&a);
    state.sets.get_mut(&a.workspace_id).unwrap().ready = 0;
    state.register(a.clone(), 101);
    state.register(b.clone(), 101);
    for _ in 0..100 {
        // RV, labels unrelated to readiness, container progress and timestamps
        // never enter this minimal cache, so these updates retain the same view.
        assert!(state.refresh_workspace(a.workspace_id, 102).is_empty());
    }
    state.sets.get_mut(&a.workspace_id).unwrap().ready = 1;
    let wake = state.refresh_workspace(a.workspace_id, 103);
    assert_eq!(wake.len(), 1);
    assert_eq!(wake[0].job.id, a.id);
    assert!(!state.cycles[&Cycle::for_job(&b)].in_flight.contains_key(&1));
}

#[test]
fn dropping_wake_batch_releases_all_submitted_items() {
    let a = job();
    let b = job();
    let c = job();
    let mut state = ready_state(&a);
    state.sets.insert(b.workspace_id, ready_set());
    state.sets.insert(c.workspace_id, ready_set());
    let a_wake = state.register(a.clone(), 101).pop().unwrap();
    let b_wake = state.register(b.clone(), 101).pop().unwrap();
    let c_wake = state.register(c.clone(), 101).pop().unwrap();
    let shared = Arc::new(Mutex::new(state));
    {
        let _cancelled = WakeBatch {
            state: shared.clone(),
            reservations: vec![a_wake, b_wake, c_wake],
        };
    }
    let state = shared.lock().unwrap();
    for job in [&a, &b, &c] {
        assert!(state.cycles[&Cycle::for_job(job)].in_flight.is_empty());
        assert_eq!(state.cycles[&Cycle::for_job(job)].retry_after.len(), 1);
    }
}

#[test]
fn aggregate_catching_up_keeps_the_pod_ready_identity() {
    let job = job();
    let mut state = ready_state(&job);
    state.sets.get_mut(&job.workspace_id).unwrap().ready = 0;
    state.pods.insert(
        job.workspace_id,
        PodView {
            uid: "pod-a".into(),
            generation: 7,
            owner_uid: "set-a".into(),
            revision: Some("revision-a".into()),
            ready: true,
        },
    );
    let wake = state.register(job.clone(), 101).pop().unwrap();
    state.complete(&wake, Ok(true), 101);
    state.sets.get_mut(&job.workspace_id).unwrap().ready = 1;
    assert!(state.refresh_workspace(job.workspace_id, 102).is_empty());
    assert!(state.register(job.clone(), 103).is_empty());
    // A current NotReady Pod prevents stale aggregate readiness from forming a hint.
    state.pods.get_mut(&job.workspace_id).unwrap().ready = false;
    assert!(state.refresh_workspace(job.workspace_id, 104).is_empty());
}

#[test]
fn replaced_ready_pod_is_a_new_edge_even_when_aggregate_stays_ready() {
    let job = job();
    let mut state = ready_state(&job);
    state.pods.insert(
        job.workspace_id,
        PodView {
            uid: "pod-a".into(),
            generation: 7,
            owner_uid: "set-a".into(),
            revision: Some("revision-a".into()),
            ready: true,
        },
    );
    let first = state.register(job.clone(), 101).pop().unwrap();
    state.complete(&first, Ok(true), 101);
    state.pods.get_mut(&job.workspace_id).unwrap().uid = "replacement-pod".into();
    let next = state
        .refresh_workspace(job.workspace_id, 102)
        .pop()
        .unwrap();
    assert_ne!(first.edge, next.edge);
}

#[test]
fn pruning_old_generations_keeps_only_unexpired_cycle_edges() {
    let mut job = job();
    let mut state = ready_state(&job);
    for generation in 1..=100 {
        job.generation = generation;
        job.payload_json = format!("generation-{generation}");
        state.sets.get_mut(&job.workspace_id).unwrap().generation = generation;
        state.register(job.clone(), 101);
    }
    assert_eq!(state.edges[&job.workspace_id].len(), 100);
    // One new cycle is still active when the earlier 100 generations expire.
    job.generation = 101;
    job.reconciled_at = 150;
    job.until = 210;
    job.payload_json = "current-generation".into();
    state.sets.get_mut(&job.workspace_id).unwrap().generation = 101;
    state.register(job.clone(), 151);
    state.prune(161);
    assert_eq!(state.cycles.len(), 1);
    assert_eq!(state.edges[&job.workspace_id].len(), 1);
    assert!(state.edges[&job.workspace_id].contains_key(&(101, 1)));
}

#[tokio::test]
async fn only_successful_nonempty_sql_wakes_notify_job_workers() {
    use crate::storage::NewJob;
    use futures::{FutureExt, poll};

    for outcome in ["success", "empty", "failure"] {
        let installation = "observer-wakeup-test".parse().unwrap();
        let database = Database::connect("sqlite::memory:", installation)
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let wakeup = Arc::new(Notify::new());
        let observer =
            WorkspaceObservations::new(database.clone(), "observer-wakeup-test".parse().unwrap())
                .with_job_wakeup(wakeup.clone());
        let now = unix_now();
        let mut parked = job();
        parked.until = now + 60;
        parked.reconciled_at = now;
        parked.payload_json = "{}".into();
        parked.id = database
            .enqueue_job(
                NewJob {
                    kind: "reconcile_workspace".into(),
                    workspace_id: None,
                    payload: json!({}),
                    available_at: parked.until,
                },
                now,
            )
            .await
            .unwrap();
        if outcome == "empty" {
            parked.payload_json = "{\"different_nonce\":true}".into();
        }
        let Database::Sqlite { pool, .. } = &database else {
            unreachable!()
        };
        if outcome == "failure" {
            sqlx::query("CREATE TRIGGER reject_wake BEFORE UPDATE ON jobs BEGIN SELECT RAISE(FAIL, 'injected wake failure'); END")
                .execute(pool).await.unwrap();
        }
        let reservations = {
            let mut state = observer.state.lock().unwrap();
            state.sets.insert(parked.workspace_id, ready_set());
            state.register(parked.clone(), now)
        };
        assert_eq!(reservations.len(), 1);
        let notified = wakeup.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        observer.deliver(reservations).await;
        assert_eq!(poll!(notified.as_mut()).is_ready(), outcome == "success");
        let available_at: i64 = sqlx::query_scalar("SELECT available_at FROM jobs WHERE id = ?1")
            .bind(parked.id.to_string())
            .fetch_one(pool)
            .await
            .unwrap();
        if outcome == "success" {
            assert!(available_at <= unix_now());
        } else {
            assert_eq!(available_at, parked.until);
        }
        assert!(
            wakeup.notified().now_or_never().is_none(),
            "broadcast must not leave a stored permit"
        );
    }
}
