use super::*;

#[test]
fn yaml_round_trip_contains_only_explicit_template_fields() {
    let mut spec = WorkspaceTemplateSpec::standard(
        "registry.example/node@sha256:test",
        AccessMode::Internal,
        Resources {
            cpu_millis: 6_000,
            memory_mib: 4_096,
            gpu_count: 0,
            disk_gib: 60,
        },
    );
    spec.workspace_user = "node-dev".to_owned();
    spec.workspace_home = "/home/node-dev".to_owned();
    let document = WorkspaceTemplateDocument::new("Node.js 开发", spec);
    let yaml = document.to_yaml().unwrap();
    assert!(yaml.contains("access_mode: internal"));
    assert!(yaml.contains("workspace_user: node-dev"));
    assert!(yaml.contains("runtime_tmp_memory_mib: 512"));
    assert!(yaml.contains("scratch_medium: disk"));
    assert!(yaml.contains("build_scratch_gib: 12"));
    assert!(yaml.contains("buildkit_cache_gib: 8"));
    assert!(yaml.contains("runtime_class_name: null"));
    assert!(yaml.contains("egress_policy: unrestricted"));
    assert_eq!(WorkspaceTemplateDocument::parse(&yaml).unwrap(), document);
    let json = serde_json::to_value(&document.spec).unwrap();
    assert_eq!(json["access_mode"], "internal");
    assert_eq!(json["workspace_user"], "node-dev");
    assert!(json.get("accessMode").is_none());
}

#[test]
fn storage_policy_defaults_scratch_medium_to_disk_for_existing_templates() {
    let yaml = r#"
apiVersion: workspace.memeloop.dev/v1
kind: WorkspaceTemplate
metadata:
  name: existing-template
spec:
  image: registry.example/dev:latest
  access_mode: internal
  resources:
    cpu_millis: 1000
    memory_mib: 1024
    gpu_count: 0
    disk_gib: 20
  pod_requests:
    cpu_millis: 1000
    memory_mib: 1024
  workspace_user: workspace
  workspace_home: /workspace
  storage_policy:
    runtime_tmp_memory_mib: 512
    build_scratch_gib: 12
    buildkit_cache_gib: 8
    codex_scratch_gib: 2
    home_reserve_mib: null
"#;
    let document = WorkspaceTemplateDocument::parse(yaml).unwrap();
    assert_eq!(
        document.spec.storage_policy.scratch_medium,
        ScratchMedium::Disk
    );
}

#[test]
fn scratch_medium_round_trips_and_rejects_unknown_values() {
    let policy = WorkspaceStoragePolicy {
        scratch_medium: ScratchMedium::Memory,
        ..WorkspaceStoragePolicy::default()
    };
    let json = serde_json::to_value(policy).unwrap();
    assert_eq!(json["scratch_medium"], "memory");
    assert_eq!(
        serde_json::from_value::<WorkspaceStoragePolicy>(json).unwrap(),
        policy
    );
    assert!(
        serde_json::from_str::<WorkspaceStoragePolicy>(r#"{"scratch_medium":"memoryy"}"#).is_err()
    );
}

#[test]
fn runtime_class_name_is_optional_and_must_be_a_dns_label() {
    let mut spec = WorkspaceTemplateSpec::standard(
        "registry.example/dev:latest",
        AccessMode::Internal,
        Resources {
            cpu_millis: 1_000,
            memory_mib: 1_024,
            gpu_count: 0,
            disk_gib: 20,
        },
    );
    spec.runtime_class_name = Some("gvisor-sandbox".to_owned());
    let document = WorkspaceTemplateDocument::new("sandbox", spec.clone());
    let yaml = document.to_yaml().unwrap();
    assert!(yaml.contains("runtime_class_name: gvisor-sandbox"));
    assert_eq!(WorkspaceTemplateDocument::parse(&yaml).unwrap(), document);

    spec.runtime_class_name = Some("GVisor".to_owned());
    assert_eq!(spec.validate(), Err(TemplateError::RuntimeClass));
    spec.runtime_class_name = Some("gvisor sandbox".to_owned());
    assert_eq!(spec.validate(), Err(TemplateError::RuntimeClass));
}

#[test]
fn rejects_requests_above_limits() {
    let mut spec = WorkspaceTemplateSpec::standard(
        "registry.example/dev:latest",
        AccessMode::Internal,
        Resources {
            cpu_millis: 1_000,
            memory_mib: 1_024,
            gpu_count: 0,
            disk_gib: 20,
        },
    );
    spec.pod_requests.cpu_millis = 1_001;
    assert_eq!(spec.validate(), Err(TemplateError::PodResources));
}

#[test]
fn rejects_workspace_identity_that_could_escape_generated_ssh_configuration() {
    let resources = Resources {
        cpu_millis: 1_000,
        memory_mib: 1_024,
        gpu_count: 0,
        disk_gib: 20,
    };
    let mut spec = WorkspaceTemplateSpec::standard(
        "registry.example/dev:latest",
        AccessMode::Internal,
        resources,
    );
    spec.workspace_user = "workspace\nPermitRootLogin yes".to_owned();
    assert_eq!(spec.validate(), Err(TemplateError::WorkspaceIdentity));
}

#[test]
fn storage_policy_requires_bounded_volumes_and_a_safe_home_reserve() {
    let mut spec = WorkspaceTemplateSpec::standard(
        "registry.example/dev:latest",
        AccessMode::Internal,
        Resources {
            cpu_millis: 1_000,
            memory_mib: 1_024,
            gpu_count: 0,
            disk_gib: 20,
        },
    );
    spec.storage_policy.home_reserve_mib = Some(0);
    assert_eq!(spec.validate(), Err(TemplateError::StoragePolicy));

    spec.storage_policy = WorkspaceStoragePolicy::default();
    spec.storage_policy.runtime_tmp_memory_mib = 0;
    assert_eq!(spec.validate(), Err(TemplateError::StoragePolicy));

    spec.storage_policy = WorkspaceStoragePolicy::default();
    spec.storage_policy.home_reserve_mib = Some(2_048);
    assert_eq!(spec.validate(), Ok(()));
    spec.storage_policy.home_reserve_mib = Some(2_049);
    assert_eq!(spec.validate(), Err(TemplateError::StoragePolicy));

    let policy = WorkspaceStoragePolicy::default();
    assert_eq!(policy.effective_home_reserve_mib(60), 1_024);
    assert_eq!(policy.effective_home_reserve_mib(5), 512);
}
