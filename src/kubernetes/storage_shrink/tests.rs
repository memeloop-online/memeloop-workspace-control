use super::*;
use crate::{
    quota::Resources,
    storage::{StorageShrinkOperation, StorageShrinkRequest},
    templates::WorkspaceTemplateSpec,
    workspace_runtime::{WorkspaceRuntimeIdentity, workspace_short_id_for},
    workspaces::{AccessMode, WorkspaceState},
};
use uuid::Uuid;

fn workspace() -> Workspace {
    let id = Uuid::now_v7();
    Workspace {
        id,
        short_id: workspace_short_id_for(id),
        organization_id: Uuid::now_v7(),
        owner_id: Uuid::now_v7(),
        name: "shrink-test".to_owned(),
        template_id: None,
        node_pool: "default".to_owned(),
        runtime: WorkspaceRuntimeIdentity,
        home_volume_binding: None,
        scratch_volume_binding: None,
        template: WorkspaceTemplateSpec::standard(
            "untrusted-workspace:latest",
            AccessMode::Internal,
            Resources {
                cpu_millis: 1000,
                memory_mib: 2048,
                gpu_count: 0,
                disk_gib: 60,
            },
        ),
        state: WorkspaceState::Ready,
        generation: 1,
        created_at: 1,
        updated_at: 1,
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

fn operation(workspace: &Workspace) -> StorageShrinkOperation {
    StorageShrinkOperation::new(
        workspace.id,
        StorageShrinkRequest {
            home_gib: Some(20),
            temporary_storage_gib: None,
            expected_generation: 1,
            reclaim_source: false,
        },
        workspace.owner_id,
        1,
    )
}

fn metadata(name: &str) -> ObjectMeta {
    ObjectMeta {
        name: Some(name.to_owned()),
        namespace: Some(crate::workspace_runtime::WORKSPACE_NAMESPACE.to_owned()),
        labels: Some(BTreeMap::new()),
        ..Default::default()
    }
}

#[test]
fn helper_has_read_only_source_and_no_workspace_credentials() {
    let workspace = workspace();
    let operation = operation(&workspace);
    let pod = migration_pod(&operation, metadata("helper"), ShrinkPodStage::Copy, &binding("source", 60), Some(&binding("destination", 20)), "reviewed.example/helper@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
    let spec = pod.spec.as_ref().unwrap();
    assert_eq!(spec.automount_service_account_token, Some(false));
    assert_eq!(spec.containers.len(), 1);
    assert!(
        !spec.containers[0]
            .image
            .as_ref()
            .unwrap()
            .contains("untrusted-workspace")
    );
    assert!(spec.containers[0].env_from.is_none());
    let source = spec
        .volumes
        .as_ref()
        .unwrap()
        .iter()
        .find(|volume| volume.name == "source")
        .unwrap();
    assert_eq!(
        source.persistent_volume_claim.as_ref().unwrap().read_only,
        Some(true)
    );
    assert!(
        spec.volumes
            .as_ref()
            .unwrap()
            .iter()
            .all(|volume| volume.secret.is_none()
                && volume.projected.is_none()
                && volume.host_path.is_none())
    );
    assert!(
        !spec.containers[0]
            .security_context
            .as_ref()
            .unwrap()
            .capabilities
            .as_ref()
            .unwrap()
            .add
            .as_ref()
            .unwrap()
            .contains(&"SYS_ADMIN".to_owned())
    );
    validate_helper_spec(&pod, &pod).unwrap();
    let mut tampered = pod.clone();
    tampered.spec.as_mut().unwrap().volumes.as_mut().unwrap()[0]
        .persistent_volume_claim
        .as_mut()
        .unwrap()
        .claim_name = "foreign-claim".to_owned();
    assert!(validate_helper_spec(&tampered, &pod).is_err());
}

fn server_defaulted_helper(mut pod: Pod) -> Pod {
    let spec = pod.spec.as_mut().unwrap();
    for variable in spec.containers[0].env.as_mut().unwrap() {
        if variable.value.as_deref() == Some("") {
            variable.value = None;
        }
    }
    for volume in spec.volumes.as_mut().unwrap() {
        if let Some(claim) = volume.persistent_volume_claim.as_mut()
            && claim.read_only == Some(false)
        {
            claim.read_only = None;
        }
    }
    for mount in spec.containers[0].volume_mounts.as_mut().unwrap() {
        if mount.read_only == Some(false) {
            mount.read_only = None;
        }
    }
    pod
}

#[test]
fn helper_manifests_survive_api_server_scalar_omission() {
    let workspace = workspace();
    let operation = operation(&workspace);
    let source = binding("source", 60);
    let destination = binding("destination", 20);
    for stage in [
        ShrinkPodStage::Preflight,
        ShrinkPodStage::Measure,
        ShrinkPodStage::Copy,
        ShrinkPodStage::Verify,
    ] {
        let target =
            matches!(stage, ShrinkPodStage::Copy | ShrinkPodStage::Verify).then_some(&destination);
        let desired = migration_pod(
            &operation,
            metadata(stage.name()),
            stage,
            &source,
            target,
            "approved-image",
        )
        .unwrap();
        let expected_bytes = target.map_or_else(
            || "0".to_owned(),
            |binding| (binding.capacity_gib * (1 << 30)).to_string(),
        );
        let spec = desired.spec.as_ref().unwrap();
        assert_eq!(
            spec.containers[0].env.as_ref().unwrap()[0].value.as_deref(),
            Some(expected_bytes.as_str())
        );
        if target.is_some() {
            let expected_read_only = matches!(stage, ShrinkPodStage::Verify).then_some(true);
            let claim = spec
                .volumes
                .as_ref()
                .unwrap()
                .iter()
                .find(|volume| volume.name == "destination")
                .unwrap()
                .persistent_volume_claim
                .as_ref()
                .unwrap();
            assert_eq!(claim.read_only, expected_read_only);
            let mount = spec.containers[0]
                .volume_mounts
                .as_ref()
                .unwrap()
                .iter()
                .find(|mount| mount.name == "destination")
                .unwrap();
            assert_eq!(mount.read_only, expected_read_only);
        }
        validate_helper_spec(&server_defaulted_helper(desired.clone()), &desired).unwrap();
    }
}

#[test]
fn canonical_helper_validation_rejects_command_env_and_security_drift() {
    let workspace = workspace();
    let operation = operation(&workspace);
    let desired = migration_pod(
        &operation,
        metadata("copy"),
        ShrinkPodStage::Copy,
        &binding("source", 60),
        Some(&binding("destination", 20)),
        "approved-image",
    )
    .unwrap();
    validate_helper_spec(&server_defaulted_helper(desired.clone()), &desired).unwrap();
    let mut tampered = server_defaulted_helper(desired.clone());
    tampered.spec.as_mut().unwrap().containers[0].command = Some(vec!["/bin/sh".to_owned()]);
    assert!(validate_helper_spec(&tampered, &desired).is_err());
    let mut tampered = server_defaulted_helper(desired.clone());
    tampered.spec.as_mut().unwrap().containers[0]
        .env
        .as_mut()
        .unwrap()[0]
        .value = Some("1".to_owned());
    assert!(validate_helper_spec(&tampered, &desired).is_err());
    let mut tampered = server_defaulted_helper(desired.clone());
    tampered.spec.as_mut().unwrap().containers[0]
        .security_context
        .as_mut()
        .unwrap()
        .privileged = Some(true);
    assert!(validate_helper_spec(&tampered, &desired).is_err());
    let mut tampered = server_defaulted_helper(desired.clone());
    tampered.spec.as_mut().unwrap().containers[0]
        .volume_mounts
        .as_mut()
        .unwrap()[0]
        .read_only = None;
    assert!(validate_helper_spec(&tampered, &desired).is_err());
}

#[test]
fn rejects_api_mutable_image_and_container_injection() {
    let workspace = workspace();
    let operation = operation(&workspace);
    let desired = migration_pod(
        &operation,
        metadata("copy"),
        ShrinkPodStage::Copy,
        &binding("source", 60),
        Some(&binding("destination", 20)),
        "approved-image",
    )
    .unwrap();
    validate_helper_spec(&server_defaulted_helper(desired.clone()), &desired).unwrap();
    let mut tampered = server_defaulted_helper(desired.clone());
    tampered.spec.as_mut().unwrap().containers[0].image =
        Some("reviewed.example/helper:swapped".to_owned());
    assert!(validate_helper_spec(&tampered, &desired).is_err());
    let injected_init: k8s_openapi::api::core::v1::Container = serde_json::from_value(json!({
        "name": "injected-init",
        "image": "reviewed.example/helper:swapped"
    }))
    .unwrap();
    let mut tampered = server_defaulted_helper(desired.clone());
    tampered.spec.as_mut().unwrap().init_containers = Some(vec![injected_init]);
    assert!(validate_helper_spec(&tampered, &desired).is_err());
    let injected_debug: k8s_openapi::api::core::v1::EphemeralContainer =
        serde_json::from_value(json!({
            "name": "injected-debug",
            "image": "reviewed.example/helper:swapped"
        }))
        .unwrap();
    let mut tampered = server_defaulted_helper(desired.clone());
    tampered.spec.as_mut().unwrap().ephemeral_containers = Some(vec![injected_debug]);
    assert!(validate_helper_spec(&tampered, &desired).is_err());
}

#[test]
fn preflight_has_no_persistent_mounts_and_verify_is_read_only() {
    let workspace = workspace();
    let operation = operation(&workspace);
    let source = binding("source", 60);
    let destination = binding("destination", 20);
    let preflight = migration_pod(
        &operation,
        metadata("preflight"),
        ShrinkPodStage::Preflight,
        &source,
        None,
        "approved-image",
    )
    .unwrap();
    assert!(
        preflight
            .spec
            .unwrap()
            .volumes
            .unwrap()
            .iter()
            .all(|volume| volume.persistent_volume_claim.is_none())
    );
    let verify = migration_pod(
        &operation,
        metadata("verify"),
        ShrinkPodStage::Verify,
        &source,
        Some(&destination),
        "approved-image",
    )
    .unwrap();
    assert!(
        verify
            .spec
            .unwrap()
            .volumes
            .unwrap()
            .iter()
            .filter_map(|volume| volume.persistent_volume_claim.as_ref())
            .all(|claim| claim.read_only == Some(true))
    );
}

#[test]
fn rejects_allow_all_or_wrong_selector_helper_network_policy() {
    let workspace = workspace();
    let operation = operation(&workspace);
    let mut policy: NetworkPolicy = serde_json::from_value(json!({"metadata": {}, "spec": {"podSelector":{"matchLabels":{OPERATION_LABEL:operation.id.to_string()}}, "policyTypes":["Ingress","Egress"]}})).unwrap();
    validate_helper_policy(&policy, &operation).unwrap();
    policy.spec.as_mut().unwrap().egress = Some(vec![Default::default()]);
    assert!(validate_helper_policy(&policy, &operation).is_err());
    policy.spec.as_mut().unwrap().egress = None;
    policy
        .spec
        .as_mut()
        .unwrap()
        .pod_selector
        .as_mut()
        .unwrap()
        .match_labels = Some(BTreeMap::new());
    assert!(validate_helper_policy(&policy, &operation).is_err());
}

#[test]
fn reclamation_checks_pv_uid_claim_uid_and_csi_handle() {
    let binding = binding("source", 60);
    let identity = StorageVolumeIdentity {
        pv_name: "source-pv".to_owned(),
        pv_uid: "pv-uid".to_owned(),
        csi_driver: "driver.longhorn.io".to_owned(),
        csi_volume_handle: "backend-volume".to_owned(),
        storage_class_name: "longhorn".to_owned(),
        longhorn_namespace: None,
        longhorn_volume_uid: None,
    };
    let volume: PersistentVolume = serde_json::from_value(json!({"metadata":{"name":"source-pv","uid":"pv-uid"},"spec":{"storageClassName":"longhorn","claimRef":{"name":binding.claim_name,"namespace":binding.namespace,"uid":binding.claim_uid},"csi":{"driver":"driver.longhorn.io","volumeHandle":"backend-volume"}}})).unwrap();
    validate_pv_identity(&volume, &binding, &identity).unwrap();
    let mut wrong_uid = volume.clone();
    wrong_uid.metadata.uid = Some("replacement-pv".to_owned());
    assert!(validate_pv_identity(&wrong_uid, &binding, &identity).is_err());
    let mut wrong_claim = volume.clone();
    wrong_claim
        .spec
        .as_mut()
        .unwrap()
        .claim_ref
        .as_mut()
        .unwrap()
        .uid = Some("foreign-claim-uid".to_owned());
    assert!(validate_pv_identity(&wrong_claim, &binding, &identity).is_err());
    let mut wrong_backend = volume;
    wrong_backend
        .spec
        .as_mut()
        .unwrap()
        .csi
        .as_mut()
        .unwrap()
        .volume_handle = "foreign-backend".to_owned();
    assert!(validate_pv_identity(&wrong_backend, &binding, &identity).is_err());
}

#[test]
fn copy_and_verification_cover_sparse_metadata_checksum_and_hardlinks() {
    let copy = ShrinkPodStage::Copy.script();
    for required in [
        "pipefail",
        "--sparse",
        "--xattrs",
        "--acls",
        "--numeric-owner",
        "--one-file-system",
        "--same-permissions",
    ] {
        assert!(copy.contains(required), "{required}");
    }
    let verify = ShrinkPodStage::Verify.script();
    for required in [
        "python3 -I",
        "hashlib.sha256",
        "st_mtime_ns",
        "st_nlink",
        "source_links",
        "destination_links",
        "os.readlink",
        "os.getxattr",
        "directory entries mismatch",
    ] {
        assert!(verify.contains(required), "{required}");
    }
}
