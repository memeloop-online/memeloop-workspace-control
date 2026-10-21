use crate::{
    quota::Resources,
    storage::{CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate},
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::{AccessMode, WorkspaceAction, WorkspaceObservation},
};

use super::*;

const WORKSPACE_IMAGE: &str = "registry.example/workspace@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const REPLACEMENT_IMAGE: &str = "registry.example/workspace@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

async fn stopped_workspace() -> (Database, Workspace, Uuid) {
    let database = Database::connect("sqlite::memory:", "storage-shrink-test".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
        .upsert_image_policy(WORKSPACE_IMAGE, true, 1)
        .await
        .unwrap();
    database
        .upsert_image_policy(REPLACEMENT_IMAGE, true, 2)
        .await
        .unwrap();
    let admin = database
        .create_user_with_initial_key(
            "Admin",
            "image-update-admin-000000000000000000000",
            true,
            crate::auth::ApiKeyScope::initial_key_defaults(true),
            31_536_000,
            1,
        )
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Storage shrink".to_owned(),
                owner_user_id: admin.user_id,
            },
            2,
        )
        .await
        .unwrap();
    let yaml = WorkspaceTemplateDocument::new(
        "Storage shrink",
        WorkspaceTemplateSpec::standard(
            WORKSPACE_IMAGE,
            AccessMode::Internal,
            Resources {
                cpu_millis: 1_000,
                memory_mib: 2_048,
                gpu_count: 0,
                disk_gib: 60,
            },
        ),
    )
    .to_yaml()
    .unwrap();
    let template = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(organization.id),
                yaml,
            },
            true,
            3,
        )
        .await
        .unwrap();
    let workspace = database
        .create_workspace(
            CreateWorkspace {
                organization_id: organization.id,
                owner_id: admin.user_id,
                name: "Stopped storage shrink".to_owned(),
                template_id: template.id,
                resources: None,
                organization_injection_refs: None,
                user_injection_refs: None,
            },
            true,
            admin.user_id,
            4,
        )
        .await
        .unwrap();
    let stopping = database
        .request_workspace_action(workspace.id, WorkspaceAction::Stop, admin.user_id, 5)
        .await
        .unwrap();
    let stopped = database
        .record_workspace_observation(
            stopping.id,
            stopping.generation,
            WorkspaceObservation::Stopped,
            admin.user_id,
            6,
        )
        .await
        .unwrap()
        .unwrap();
    (database, stopped, admin.user_id)
}

fn request(workspace: &Workspace) -> StorageShrinkRequest {
    StorageShrinkRequest {
        home_gib: Some(20),
        temporary_storage_gib: Some(8),
        expected_generation: workspace.generation,
        reclaim_source: false,
    }
}

fn binding(name: &str, capacity_gib: u64) -> WorkspaceHomeVolumeBinding {
    WorkspaceHomeVolumeBinding {
        namespace: crate::workspace_runtime::WORKSPACE_NAMESPACE.to_owned(),
        claim_name: name.to_owned(),
        claim_uid: format!("uid-{name}"),
        capacity_gib,
    }
}

fn identity(name: &str) -> StorageVolumeIdentity {
    StorageVolumeIdentity {
        pv_name: name.to_owned(),
        pv_uid: format!("pv-uid-{name}"),
        csi_driver: "driver.longhorn.io".to_owned(),
        csi_volume_handle: format!("handle-{name}"),
        storage_class_name: "longhorn".to_owned(),
        longhorn_namespace: Some("longhorn-system".to_owned()),
        longhorn_volume_uid: Some(format!("backend-{name}")),
    }
}

fn verified(operation: &mut StorageShrinkOperation) {
    for (selected, role, old_size, new_size) in [
        (&mut operation.home, "home", 60, 20),
        (&mut operation.temporary, "temporary", 10, 8),
    ] {
        selected.source = Some(binding(&format!("old-{role}"), old_size));
        selected.destination = Some(binding(&format!("new-{role}"), new_size));
        selected.source_volume = Some(identity(&format!("old-{role}")));
        selected.destination_volume = Some(identity(&format!("new-{role}")));
        selected.used_bytes = Some(2 << 30);
        selected.used_inodes = Some(500);
        selected.verified = true;
    }
}

#[tokio::test]
async fn validates_targets_and_generation_without_changing_workspace() {
    let (database, stopped, _) = stopped_workspace().await;
    for target in [0, 60, 61, u64::MAX] {
        let mut invalid = request(&stopped);
        invalid.home_gib = Some(target);
        assert!(matches!(
            invalid.validate(&stopped),
            Err(StorageError::InvalidWorkspaceStorageShrink)
        ));
    }
    let mut invalid = request(&stopped);
    invalid.expected_generation += 1;
    assert!(matches!(
        invalid.validate(&stopped),
        Err(StorageError::WorkspaceStorageShrinkConflict)
    ));
    assert_eq!(
        database.get_workspace(stopped.id).await.unwrap().generation,
        stopped.generation
    );
}

#[tokio::test]
async fn existing_resize_rejects_bound_scratch_capacity_changes_without_mutation() {
    let (database, stopped, actor) = stopped_workspace().await;
    let mut shrink_request = request(&stopped);
    shrink_request.temporary_storage_gib = Some(5);
    let mut operation = database
        .enqueue_storage_shrink(stopped.id, shrink_request, actor, 7)
        .await
        .unwrap();
    verified(&mut operation);
    operation.temporary.destination = Some(binding("new-temporary", 5));
    operation.phase = StorageShrinkPhase::Completed;
    operation.switched = true;
    operation.reclaimable = true;
    database
        .save_storage_shrink(&operation, ShrinkWorkspaceUpdate::Switch)
        .await
        .unwrap();
    let bound = database.get_workspace(stopped.id).await.unwrap();
    assert_eq!(bound.state, WorkspaceState::Stopped);
    assert_eq!(
        bound.scratch_volume_binding.as_ref().unwrap().capacity_gib,
        5
    );
    let before = database.export_snapshot(8).await.unwrap();
    for target in [8, 3] {
        assert!(matches!(
            database
                .update_stopped_workspace_temporary_storage(
                    stopped.id,
                    target,
                    bound.generation,
                    actor,
                    9
                )
                .await,
            Err(StorageError::WorkspaceTemporaryStorageUpdateConflict)
        ));
        let unchanged = database.get_workspace(stopped.id).await.unwrap();
        assert_eq!(unchanged.state, bound.state);
        assert_eq!(unchanged.generation, bound.generation);
        assert_eq!(unchanged.updated_at, bound.updated_at);
        assert_eq!(
            unchanged.scratch_volume_binding,
            bound.scratch_volume_binding
        );
        assert_eq!(unchanged.template.storage_policy.temporary_storage_gib, 5);
        let after = database.export_snapshot(10).await.unwrap();
        assert_eq!(after.tables, before.tables);
    }
    let same_size = database
        .update_stopped_workspace_temporary_storage(stopped.id, 5, bound.generation, actor, 11)
        .await
        .unwrap();
    assert_eq!(same_size.state, WorkspaceState::Stopped);
    assert_eq!(same_size.generation, bound.generation + 1);
    assert_eq!(
        same_size.scratch_volume_binding,
        bound.scratch_volume_binding
    );
    assert_eq!(same_size.template.storage_policy.temporary_storage_gib, 5);
}

#[tokio::test]
async fn existing_resize_preserves_capacity_changes_for_unbound_scratch() {
    let (database, mut stopped, actor) = stopped_workspace().await;
    assert!(stopped.scratch_volume_binding.is_none());
    for target in [8, 3] {
        let previous_generation = stopped.generation;
        stopped = database
            .update_stopped_workspace_temporary_storage(
                stopped.id,
                target,
                previous_generation,
                actor,
                8,
            )
            .await
            .unwrap();
        assert_eq!(stopped.state, WorkspaceState::Stopped);
        assert_eq!(stopped.generation, previous_generation + 1);
        assert!(stopped.scratch_volume_binding.is_none());
        assert_eq!(
            stopped.template.storage_policy.temporary_storage_gib,
            target
        );
        let persisted = database.get_workspace(stopped.id).await.unwrap();
        assert_eq!(
            persisted.template.storage_policy.temporary_storage_gib,
            target
        );
        let snapshot = database.export_snapshot(9).await.unwrap();
        let yaml = snapshot.tables["workspaces"][0]["template_snapshot_yaml"]
            .as_str()
            .unwrap();
        let document = WorkspaceTemplateDocument::parse(yaml).unwrap();
        assert_eq!(document.spec.storage_policy.temporary_storage_gib, target);
    }
}

#[tokio::test]
async fn reserves_job_atomically_and_replays_identical_requests() {
    let (database, stopped, actor) = stopped_workspace().await;
    let operation = database
        .enqueue_storage_shrink(stopped.id, request(&stopped), actor, 7)
        .await
        .unwrap();
    let replay = database
        .enqueue_storage_shrink(stopped.id, request(&stopped), actor, 8)
        .await
        .unwrap();
    assert_eq!(operation.id, replay.id);
    assert_eq!(
        database.get_workspace(stopped.id).await.unwrap().generation,
        stopped.generation + 1
    );
    assert!(database.storage_shrink_active(stopped.id).await.unwrap());
    let Database::Sqlite { pool, .. } = &database else {
        unreachable!()
    };
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE kind = 'workspace_storage_shrink'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    let mut conflict = request(&stopped);
    conflict.home_gib = Some(19);
    assert!(matches!(
        database
            .enqueue_storage_shrink(stopped.id, conflict, actor, 9)
            .await,
        Err(StorageError::WorkspaceStorageShrinkConflict)
    ));
}

#[tokio::test]
async fn active_operation_blocks_lifecycle_and_all_storage_mutations() {
    let (database, stopped, actor) = stopped_workspace().await;
    database
        .enqueue_storage_shrink(stopped.id, request(&stopped), actor, 7)
        .await
        .unwrap();
    let current = database.get_workspace(stopped.id).await.unwrap();
    for action in [
        WorkspaceAction::Start,
        WorkspaceAction::Stop,
        WorkspaceAction::Restart,
        WorkspaceAction::Delete,
    ] {
        assert!(matches!(
            database
                .request_workspace_action(stopped.id, action, actor, 8)
                .await,
            Err(StorageError::WorkspaceStorageShrinkConflict)
        ));
    }
    assert!(matches!(
        database
            .update_stopped_workspace_temporary_storage(stopped.id, 8, current.generation, actor, 8)
            .await,
        Err(StorageError::WorkspaceStorageShrinkConflict)
    ));
    assert!(matches!(
        database
            .bind_workspace_home_volume(
                stopped.id,
                &binding("new-home", 20),
                current.generation,
                actor,
                8,
                false
            )
            .await,
        Err(StorageError::WorkspaceStorageShrinkConflict)
    ));
    assert!(matches!(
        database
            .update_stopped_workspace_image(
                stopped.id,
                REPLACEMENT_IMAGE,
                current.generation,
                actor,
                8
            )
            .await,
        Err(StorageError::WorkspaceStorageShrinkConflict)
    ));
}

#[tokio::test]
async fn verified_swap_updates_both_bindings_and_sizes_in_one_transaction() {
    let (database, stopped, actor) = stopped_workspace().await;
    let mut operation = database
        .enqueue_storage_shrink(stopped.id, request(&stopped), actor, 7)
        .await
        .unwrap();
    verified(&mut operation);
    operation.home.verified = false;
    assert!(matches!(
        database
            .save_storage_shrink(&operation, ShrinkWorkspaceUpdate::Switch)
            .await,
        Err(StorageError::InvalidWorkspaceStorageShrink)
    ));
    assert_eq!(
        database
            .get_workspace(stopped.id)
            .await
            .unwrap()
            .template
            .resources
            .disk_gib,
        60
    );
    operation.home.verified = true;
    operation.phase = StorageShrinkPhase::Resuming;
    operation.switched = true;
    database
        .save_storage_shrink(&operation, ShrinkWorkspaceUpdate::Switch)
        .await
        .unwrap();
    let updated = database.get_workspace(stopped.id).await.unwrap();
    assert_eq!(updated.template.resources.disk_gib, 20);
    assert_eq!(updated.template.storage_policy.temporary_storage_gib, 8);
    assert_eq!(updated.home_volume_binding, operation.home.destination);
    assert_eq!(
        updated.scratch_volume_binding,
        operation.temporary.destination
    );
    assert_eq!(updated.state, WorkspaceState::Stopped);
    assert!(database.storage_shrink_active(stopped.id).await.unwrap());
}

#[tokio::test]
async fn failed_offline_copy_restores_source_bindings_and_releases_guard() {
    let (database, stopped, actor) = stopped_workspace().await;
    let mut operation = database
        .enqueue_storage_shrink(stopped.id, request(&stopped), actor, 7)
        .await
        .unwrap();
    verified(&mut operation);
    operation.phase = StorageShrinkPhase::Failed;
    operation.error = Some("checksum mismatch".to_owned());
    database
        .save_storage_shrink(&operation, ShrinkWorkspaceUpdate::Rollback)
        .await
        .unwrap();
    let restored = database.get_workspace(stopped.id).await.unwrap();
    assert_eq!(restored.home_volume_binding, operation.home.source);
    assert_eq!(restored.scratch_volume_binding, operation.temporary.source);
    assert_eq!(restored.template.resources.disk_gib, 60);
    assert_eq!(restored.state, WorkspaceState::Stopped);
    assert!(!database.storage_shrink_active(stopped.id).await.unwrap());
    database
        .request_workspace_action(stopped.id, WorkspaceAction::Start, actor, 8)
        .await
        .unwrap();
}

#[tokio::test]
async fn confirmation_is_idempotent_and_reclaimed_state_releases_guard() {
    let (database, stopped, actor) = stopped_workspace().await;
    let mut operation = database
        .enqueue_storage_shrink(stopped.id, request(&stopped), actor, 7)
        .await
        .unwrap();
    verified(&mut operation);
    operation.phase = StorageShrinkPhase::Completed;
    operation.switched = true;
    operation.reclaimable = true;
    database
        .save_storage_shrink(&operation, ShrinkWorkspaceUpdate::Switch)
        .await
        .unwrap();
    let confirmed = database
        .confirm_storage_shrink(stopped.id, operation.id, actor, 8)
        .await
        .unwrap();
    assert_eq!(confirmed.phase, StorageShrinkPhase::Reclaiming);
    assert!(database.storage_shrink_active(stopped.id).await.unwrap());
    let replay = database
        .confirm_storage_shrink(stopped.id, operation.id, actor, 9)
        .await
        .unwrap();
    assert_eq!(replay.id, confirmed.id);
    operation.phase = StorageShrinkPhase::Reclaimed;
    operation.sources_retained = false;
    operation.reclaimable = false;
    database
        .save_storage_shrink(&operation, ShrinkWorkspaceUpdate::None)
        .await
        .unwrap();
    assert!(!database.storage_shrink_active(stopped.id).await.unwrap());
    let reclaimed_replay = database
        .confirm_storage_shrink(stopped.id, operation.id, actor, 10)
        .await
        .unwrap();
    assert_eq!(reclaimed_replay.phase, StorageShrinkPhase::Reclaimed);
    database
        .request_workspace_action(stopped.id, WorkspaceAction::Start, actor, 10)
        .await
        .unwrap();
}

#[tokio::test]
async fn snapshots_preserve_operation_and_explicit_scratch_binding() {
    let (database, stopped, actor) = stopped_workspace().await;
    let mut operation = database
        .enqueue_storage_shrink(stopped.id, request(&stopped), actor, 7)
        .await
        .unwrap();
    verified(&mut operation);
    operation.phase = StorageShrinkPhase::Stopping;
    database
        .save_storage_shrink(&operation, ShrinkWorkspaceUpdate::Stopping)
        .await
        .unwrap();
    let snapshot = database.export_snapshot(8).await.unwrap();
    assert_eq!(snapshot.tables["workspace_storage_shrinks"].len(), 1);
    assert!(snapshot.tables["workspaces"][0]["scratch_volume_binding_json"].is_string());
}

#[tokio::test]
async fn retained_backup_must_be_reclaimed_before_another_shrink() {
    let (database, stopped, actor) = stopped_workspace().await;
    let mut operation = database
        .enqueue_storage_shrink(stopped.id, request(&stopped), actor, 7)
        .await
        .unwrap();
    verified(&mut operation);
    operation.phase = StorageShrinkPhase::Completed;
    operation.switched = true;
    operation.reclaimable = true;
    database
        .save_storage_shrink(&operation, ShrinkWorkspaceUpdate::Switch)
        .await
        .unwrap();
    let current = database.get_workspace(stopped.id).await.unwrap();
    let next = StorageShrinkRequest {
        home_gib: Some(15),
        temporary_storage_gib: None,
        expected_generation: current.generation,
        reclaim_source: true,
    };
    assert!(matches!(
        database
            .enqueue_storage_shrink(stopped.id, next, actor, 8)
            .await,
        Err(StorageError::WorkspaceStorageShrinkConflict)
    ));
}

#[tokio::test]
async fn home_only_swap_preserves_retained_scratch_contents_and_capacity() {
    let (database, stopped, actor) = stopped_workspace().await;
    let mut home_only = request(&stopped);
    home_only.temporary_storage_gib = None;
    let mut operation = database
        .enqueue_storage_shrink(stopped.id, home_only, actor, 7)
        .await
        .unwrap();
    verified(&mut operation);
    operation.temporary.destination = None;
    operation.temporary.destination_volume = None;
    operation.temporary.verified = false;
    operation.phase = StorageShrinkPhase::Stopping;
    database
        .save_storage_shrink(&operation, ShrinkWorkspaceUpdate::Stopping)
        .await
        .unwrap();
    let stopping = database.get_workspace(stopped.id).await.unwrap();
    assert_eq!(stopping.scratch_volume_binding, operation.temporary.source);
    operation.phase = StorageShrinkPhase::Resuming;
    operation.switched = true;
    database
        .save_storage_shrink(&operation, ShrinkWorkspaceUpdate::Switch)
        .await
        .unwrap();
    let switched = database.get_workspace(stopped.id).await.unwrap();
    assert_eq!(switched.scratch_volume_binding, operation.temporary.source);
    assert_eq!(switched.template.storage_policy.temporary_storage_gib, 10);
    assert_eq!(switched.home_volume_binding, operation.home.destination);
}

#[tokio::test]
async fn deletion_persists_missing_pv_identity_without_mutating_operation_state() {
    let (database, stopped, actor) = stopped_workspace().await;
    let mut operation = database
        .enqueue_storage_shrink(stopped.id, request(&stopped), actor, 7)
        .await
        .unwrap();
    verified(&mut operation);
    let captured_identity = operation.home.destination_volume.take().unwrap();
    operation.phase = StorageShrinkPhase::Failed;
    database
        .save_storage_shrink(&operation, ShrinkWorkspaceUpdate::Rollback)
        .await
        .unwrap();
    operation.home.destination_volume = Some(captured_identity.clone());
    assert!(matches!(
        database
            .save_storage_shrink_cleanup_identity(&operation)
            .await,
        Err(StorageError::WorkspaceStorageShrinkConflict)
    ));
    database
        .request_workspace_action(stopped.id, WorkspaceAction::Delete, actor, 8)
        .await
        .unwrap();
    database
        .save_storage_shrink_cleanup_identity(&operation)
        .await
        .unwrap();
    let persisted = database
        .latest_storage_shrink(stopped.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(persisted.home.destination_volume, Some(captured_identity));
    assert_eq!(persisted.phase, StorageShrinkPhase::Failed);
    assert!(!database.storage_shrink_active(stopped.id).await.unwrap());
    operation.home.destination.as_mut().unwrap().claim_uid = "foreign-uid".to_owned();
    assert!(matches!(
        database
            .save_storage_shrink_cleanup_identity(&operation)
            .await,
        Err(StorageError::WorkspaceStorageShrinkConflict)
    ));
}

async fn retry_fixture(
    home: bool,
    temporary: bool,
    mutate: impl FnOnce(&mut StorageShrinkOperation),
) -> (Database, Workspace, StorageShrinkOperation, Uuid) {
    let (database, stopped, actor) = stopped_workspace().await;
    let mut selected = request(&stopped);
    if !home {
        selected.home_gib = None;
    }
    if !temporary {
        selected.temporary_storage_gib = None;
    }
    let mut operation = database
        .enqueue_storage_shrink(stopped.id, selected, actor, 7)
        .await
        .unwrap();
    verified(&mut operation);
    for volume in [&mut operation.home, &mut operation.temporary] {
        if volume.target_gib.is_none() {
            volume.destination = None;
            volume.destination_volume = None;
            volume.verified = false;
        }
    }
    operation.resume = true;
    operation.switched = true;
    operation.phase = StorageShrinkPhase::Resuming;
    database
        .save_storage_shrink(&operation, ShrinkWorkspaceUpdate::Switch)
        .await
        .unwrap();
    database
        .save_storage_shrink(&operation, ShrinkWorkspaceUpdate::Resume)
        .await
        .unwrap();
    let ready = database
        .record_workspace_observation(
            stopped.id,
            database.get_workspace(stopped.id).await.unwrap().generation,
            WorkspaceObservation::Ready,
            actor,
            8,
        )
        .await
        .unwrap()
        .unwrap();
    operation.phase = StorageShrinkPhase::Failed;
    operation.error = Some("Kubernetes SendRequest during resume".to_owned());
    mutate(&mut operation);
    database
        .save_storage_shrink(&operation, ShrinkWorkspaceUpdate::None)
        .await
        .unwrap();
    while let Some(job) = database
        .claim_job("retry-fixture", 10, std::time::Duration::from_secs(60))
        .await
        .unwrap()
    {
        database
            .complete_job(job.id, "retry-fixture", 10)
            .await
            .unwrap();
    }
    (database, ready, operation, actor)
}

async fn assert_retry_conflict_unchanged(
    database: &Database,
    workspace_id: Uuid,
    operation_id: Uuid,
    actor: Uuid,
) {
    let before = database.export_snapshot(20).await.unwrap();
    assert!(matches!(
        database
            .retry_storage_shrink_resume(workspace_id, operation_id, actor, 21)
            .await,
        Err(StorageError::WorkspaceStorageShrinkConflict)
    ));
    let after = database.export_snapshot(22).await.unwrap();
    assert_eq!(before.tables, after.tables);
}

#[tokio::test]
async fn retry_resumes_same_operation_without_rebinding_or_reclaiming() {
    for (home, temporary) in [(true, true), (true, false), (false, true)] {
        let (database, ready, operation, actor) = retry_fixture(home, temporary, |_| {}).await;
        let retried = database
            .retry_storage_shrink_resume(ready.id, operation.id, actor, 20)
            .await
            .unwrap();
        let mut expected = operation.clone();
        expected.phase = StorageShrinkPhase::Resuming;
        expected.error = None;
        expected.updated_at = 20;
        assert_eq!(
            serde_json::to_value(&retried).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        let mut expected_workspace = ready.clone();
        expected_workspace.generation += 1;
        expected_workspace.updated_at = 20;
        assert_eq!(
            database.get_workspace(ready.id).await.unwrap(),
            expected_workspace
        );
        assert!(database.storage_shrink_active(ready.id).await.unwrap());
        let snapshot = database.export_snapshot(20).await.unwrap();
        let pending: Vec<_> = snapshot.tables["jobs"]
            .iter()
            .filter(|job| job["status"] == "pending")
            .collect();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0]["kind"], "workspace_storage_shrink");
        let payload: serde_json::Value =
            serde_json::from_str(pending[0]["payload_json"].as_str().unwrap()).unwrap();
        assert_eq!(payload, serde_json::json!({"operation_id": operation.id}));
        let audit = snapshot.tables["audit_log"]
            .iter()
            .find(|entry| entry["action"] == "workspace.storage_shrink_resume_retried")
            .unwrap();
        assert_eq!(audit["actor_user_id"], actor.to_string());
        let metadata: serde_json::Value =
            serde_json::from_str(audit["metadata_json"].as_str().unwrap()).unwrap();
        assert_eq!(metadata["operation_id"], operation.id.to_string());
        assert_eq!(metadata["prior_error"], operation.error.unwrap());
        assert_retry_conflict_unchanged(&database, ready.id, operation.id, actor).await;
    }
}

#[tokio::test]
async fn retry_rejects_every_other_phase_and_wrong_operation_id() {
    for phase in [
        StorageShrinkPhase::Queued,
        StorageShrinkPhase::Stopping,
        StorageShrinkPhase::Measuring,
        StorageShrinkPhase::Copying,
        StorageShrinkPhase::Verifying,
        StorageShrinkPhase::Switching,
        StorageShrinkPhase::Resuming,
        StorageShrinkPhase::RollingBack,
        StorageShrinkPhase::Completed,
        StorageShrinkPhase::Reclaiming,
        StorageShrinkPhase::Reclaimed,
    ] {
        let (database, ready, operation, actor) =
            retry_fixture(true, true, |operation| operation.phase = phase).await;
        assert_retry_conflict_unchanged(&database, ready.id, operation.id, actor).await;
    }
    let (database, ready, _, actor) = retry_fixture(true, true, |_| {}).await;
    assert_retry_conflict_unchanged(&database, ready.id, Uuid::now_v7(), actor).await;
}

#[tokio::test]
async fn retry_rejects_unsafe_flags_bindings_verification_and_volume_identities() {
    let mutations: &[fn(&mut StorageShrinkOperation)] = &[
        |operation| operation.switched = false,
        |operation| operation.sources_retained = false,
        |operation| operation.reclaimable = true,
        |operation| operation.resume = false,
        |operation| operation.request.reclaim_source = true,
        |operation| operation.home.verified = false,
        |operation| operation.temporary.verified = false,
        |operation| operation.home.source = None,
        |operation| operation.temporary.destination = None,
        |operation| operation.home.source.as_mut().unwrap().claim_uid.clear(),
        |operation| {
            operation.temporary.source.as_mut().unwrap().claim_uid = operation
                .temporary
                .destination
                .as_ref()
                .unwrap()
                .claim_uid
                .clone();
        },
        |operation| {
            operation.home.destination.as_mut().unwrap().claim_uid = "foreign-uid".to_owned();
        },
        |operation| {
            operation.home.destination.as_mut().unwrap().claim_name = "foreign-claim".to_owned();
        },
        |operation| {
            operation
                .temporary
                .destination
                .as_mut()
                .unwrap()
                .capacity_gib += 1
        },
        |operation| operation.home.source.as_mut().unwrap().namespace = "foreign".to_owned(),
        |operation| operation.home.destination.as_mut().unwrap().namespace = "foreign".to_owned(),
        |operation| operation.home.target_gib = Some(19),
        |operation| operation.request.temporary_storage_gib = None,
        |operation| operation.original_home_gib += 1,
        |operation| operation.home.source_volume = None,
        |operation| operation.temporary.destination_volume = None,
        |operation| {
            operation
                .home
                .source_volume
                .as_mut()
                .unwrap()
                .pv_uid
                .clear()
        },
        |operation| {
            operation
                .home
                .destination_volume
                .as_mut()
                .unwrap()
                .pv_name
                .clear()
        },
        |operation| {
            operation
                .home
                .source_volume
                .as_mut()
                .unwrap()
                .csi_driver
                .clear()
        },
        |operation| {
            operation
                .home
                .destination_volume
                .as_mut()
                .unwrap()
                .csi_volume_handle
                .clear()
        },
        |operation| {
            operation
                .home
                .source_volume
                .as_mut()
                .unwrap()
                .storage_class_name
                .clear()
        },
        |operation| {
            operation.home.source_volume.as_mut().unwrap().pv_uid = operation
                .home
                .destination_volume
                .as_ref()
                .unwrap()
                .pv_uid
                .clone();
        },
        |operation| {
            operation.home.source_volume.as_mut().unwrap().pv_name = operation
                .home
                .destination_volume
                .as_ref()
                .unwrap()
                .pv_name
                .clone();
        },
        |operation| {
            operation
                .home
                .source_volume
                .as_mut()
                .unwrap()
                .csi_volume_handle = operation
                .home
                .destination_volume
                .as_ref()
                .unwrap()
                .csi_volume_handle
                .clone();
        },
    ];
    for mutate in mutations {
        let (database, ready, operation, actor) = retry_fixture(true, true, mutate).await;
        assert_retry_conflict_unchanged(&database, ready.id, operation.id, actor).await;
    }
}

#[tokio::test]
async fn retry_rejects_changes_to_unselected_volumes() {
    for home in [false, true] {
        for change_binding in [false, true] {
            let (database, ready, operation, actor) = retry_fixture(home, !home, |operation| {
                let unselected = if home {
                    &mut operation.temporary
                } else {
                    &mut operation.home
                };
                if change_binding {
                    unselected.source.as_mut().unwrap().claim_uid = "foreign-uid".to_owned();
                } else {
                    unselected.destination = Some(binding("unexpected-destination", 5));
                }
            })
            .await;
            assert_retry_conflict_unchanged(&database, ready.id, operation.id, actor).await;
        }
    }
}

#[tokio::test]
async fn retry_rejects_not_ready_and_latest_or_active_competition() {
    let (database, ready, operation, actor) = retry_fixture(true, true, |_| {}).await;
    let stopping = database
        .request_workspace_action(ready.id, WorkspaceAction::Stop, actor, 11)
        .await
        .unwrap();
    assert_retry_conflict_unchanged(&database, ready.id, operation.id, actor).await;
    database
        .record_workspace_observation(
            ready.id,
            stopping.generation,
            WorkspaceObservation::Stopped,
            actor,
            12,
        )
        .await
        .unwrap()
        .unwrap();
    assert_retry_conflict_unchanged(&database, ready.id, operation.id, actor).await;

    for created_at in [1, 11] {
        let (database, ready, operation, actor) = retry_fixture(true, true, |_| {}).await;
        database
            .enqueue_storage_shrink(
                ready.id,
                StorageShrinkRequest {
                    home_gib: Some(15),
                    temporary_storage_gib: None,
                    expected_generation: ready.generation,
                    reclaim_source: false,
                },
                actor,
                created_at,
            )
            .await
            .unwrap();
        assert_retry_conflict_unchanged(&database, ready.id, operation.id, actor).await;
    }
}

#[tokio::test]
async fn retry_rejects_pending_and_running_workspace_jobs() {
    for kind in ["workspace_storage_shrink", "reconcile_workspace"] {
        for running in [false, true] {
            let (database, ready, operation, actor) = retry_fixture(true, true, |_| {}).await;
            let job_id = database
                .enqueue_job(
                    crate::storage::NewJob {
                        kind: kind.to_owned(),
                        workspace_id: Some(ready.id),
                        payload: serde_json::json!({"operation_id": operation.id}),
                        available_at: 11,
                    },
                    11,
                )
                .await
                .unwrap();
            if running {
                let claimed = database
                    .claim_job("competitor", 11, std::time::Duration::from_secs(60))
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(claimed.id, job_id);
            }
            assert_retry_conflict_unchanged(&database, ready.id, operation.id, actor).await;
        }
    }
}

#[tokio::test]
async fn retry_rolls_back_operation_activation_when_job_insert_fails() {
    let (database, ready, operation, actor) = retry_fixture(false, true, |_| {}).await;
    let Database::Sqlite { pool, .. } = &database else {
        unreachable!()
    };
    sqlx::query("CREATE TRIGGER reject_retry_job BEFORE INSERT ON jobs WHEN NEW.kind = 'workspace_storage_shrink' BEGIN SELECT RAISE(ABORT, 'injected enqueue failure'); END")
        .execute(pool)
        .await
        .unwrap();
    let before = database.export_snapshot(20).await.unwrap();
    assert!(
        database
            .retry_storage_shrink_resume(ready.id, operation.id, actor, 21)
            .await
            .is_err()
    );
    assert_eq!(
        before.tables,
        database.export_snapshot(22).await.unwrap().tables
    );
}

#[tokio::test]
async fn failed_operation_still_cannot_confirm_and_retry_replays_by_idempotency() {
    let (database, ready, operation, actor) = retry_fixture(false, true, |_| {}).await;
    let before = database.export_snapshot(20).await.unwrap();
    assert!(matches!(
        database
            .confirm_storage_shrink(ready.id, operation.id, actor, 20)
            .await,
        Err(StorageError::WorkspaceStorageShrinkConflict)
    ));
    assert_eq!(
        before.tables,
        database.export_snapshot(20).await.unwrap().tables
    );
    let scope = format!("{actor}:workspace-storage-shrink-retry");
    assert_eq!(
        database
            .begin_idempotency(&scope, "retry-key", "retry-hash", 20, 100)
            .await
            .unwrap(),
        crate::storage::IdempotencyDecision::Reserved
    );
    let retried = database
        .retry_storage_shrink_resume(ready.id, operation.id, actor, 20)
        .await
        .unwrap();
    let json = serde_json::to_string(&retried).unwrap();
    database
        .finish_idempotency(&scope, "retry-key", "retry-hash", 202, &json)
        .await
        .unwrap();
    let before_replay = database.export_snapshot(21).await.unwrap();
    let replay = database
        .begin_idempotency(&scope, "retry-key", "retry-hash", 21, 100)
        .await
        .unwrap();
    assert_eq!(
        replay,
        crate::storage::IdempotencyDecision::Replay(crate::storage::IdempotencyReplay {
            status_code: 202,
            response_json: json,
        })
    );
    assert_eq!(
        before_replay.tables,
        database.export_snapshot(22).await.unwrap().tables
    );
    for other in [
        "workspace-storage-shrink",
        "workspace-storage-shrink-confirm",
    ] {
        assert_eq!(
            database
                .begin_idempotency(
                    &format!("{actor}:{other}"),
                    "retry-key",
                    "retry-hash",
                    22,
                    100
                )
                .await
                .unwrap(),
            crate::storage::IdempotencyDecision::Reserved
        );
    }
}
