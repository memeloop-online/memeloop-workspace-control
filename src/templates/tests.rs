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
    assert!(yaml.contains("temporary_storage_gib: 10"));
    assert!(yaml.contains("allowed_node_pools:"));
    assert!(yaml.contains("default_node_pool: default"));
    assert!(yaml.contains("runtime_class_name: null"));
    assert!(yaml.contains("egress_policy: unrestricted"));
    assert_eq!(WorkspaceTemplateDocument::parse(&yaml).unwrap(), document);
    let json = serde_json::to_value(&document.spec).unwrap();
    assert_eq!(json["access_mode"], "internal");
    assert_eq!(json["workspace_user"], "node-dev");
    assert!(json.get("accessMode").is_none());
}

#[test]
fn storage_and_placement_defaults_are_neutral() {
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
    temporary_storage_gib: 30
"#;
    let document = WorkspaceTemplateDocument::parse(yaml).unwrap();
    assert_eq!(document.spec.storage_policy.temporary_storage_gib, 30);
    assert_eq!(document.spec.placement, WorkspacePlacement::default());
}

#[test]
fn desktop_endpoint_is_optional_and_validated_as_a_browser_http_port() {
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
    assert_eq!(spec.desktop, None);
    spec.desktop = Some(DesktopEndpoint {
        internal_port: 6901,
        display_name: Some("Browser desktop".to_owned()),
    });
    let document = WorkspaceTemplateDocument::new("desktop", spec.clone());
    assert_eq!(
        WorkspaceTemplateDocument::parse(&document.to_yaml().unwrap()).unwrap(),
        document
    );

    for port in [80, 443, 1023, 22, 2222, 7681, 8080, 8081, 8443, 3389] {
        spec.desktop.as_mut().unwrap().internal_port = port;
        assert_eq!(spec.validate(), Err(TemplateError::Desktop), "port {port}");
    }
}

#[test]
fn storage_policy_round_trips_and_rejects_removed_fields() {
    let policy = WorkspaceStoragePolicy {
        temporary_storage_gib: 64,
    };
    let json = serde_json::to_value(policy).unwrap();
    assert_eq!(json["temporary_storage_gib"], 64);
    assert_eq!(
        serde_json::from_value::<WorkspaceStoragePolicy>(json).unwrap(),
        policy
    );
    assert!(
        serde_json::from_str::<WorkspaceStoragePolicy>(
            r#"{"temporary_storage_gib":22,"scratch":"memory"}"#
        )
        .is_err()
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
fn storage_policy_requires_a_bounded_total_and_valid_placement() {
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
    spec.storage_policy.temporary_storage_gib = 0;
    assert_eq!(spec.validate(), Err(TemplateError::StoragePolicy));

    spec.storage_policy = WorkspaceStoragePolicy::default();
    spec.storage_policy.temporary_storage_gib = 2_049;
    assert_eq!(spec.validate(), Err(TemplateError::StoragePolicy));

    spec.storage_policy = WorkspaceStoragePolicy::default();
    spec.placement.allowed_node_pools.push("gpu".to_owned());
    spec.placement.default_node_pool = "gpu".to_owned();
    assert_eq!(spec.validate(), Ok(()));
    spec.placement.allowed_node_pools.push("gpu".to_owned());
    assert_eq!(spec.validate(), Err(TemplateError::Placement));
}
