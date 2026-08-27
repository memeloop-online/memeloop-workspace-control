use memeloop_workspace_control::{
    injections::{InjectionItem, InjectionKind, InjectionValue, resolve_injections},
    kubernetes::{
        BuildError, OWNER_INSTALLATION_LABEL, OwnershipError, ResourceBuilder,
        WorkspaceResourceSpec,
    },
    quota::Resources,
    workspaces::{AccessMode, WorkspaceState},
};
use std::collections::BTreeMap;
use uuid::Uuid;

fn builder() -> ResourceBuilder {
    ResourceBuilder {
        installation_id: "public-a".parse().unwrap(),
        ttyd_image: "tsl0922/ttyd:1.7.7".to_owned(),
        higress_namespace: "higress-system".to_owned(),
        higress_pod_labels: std::collections::BTreeMap::from([(
            "app.kubernetes.io/name".to_owned(),
            "higress-gateway".to_owned(),
        )]),
        jump_host_namespace: "workspace-access".to_owned(),
        jump_host_pod_labels: std::collections::BTreeMap::from([(
            "app.kubernetes.io/name".to_owned(),
            "mwc-ssh-jump".to_owned(),
        )]),
        storage_class_name: Some("managed-delete".to_owned()),
        web_shell_domain: Some("shell.example.com".to_owned()),
        higress_gateway_name: "higress-gateway".to_owned(),
        higress_https_section_name: "https".to_owned(),
    }
}

fn workspace(state: WorkspaceState) -> WorkspaceResourceSpec {
    WorkspaceResourceSpec {
        id: Uuid::now_v7(),
        short_id: "01jabc".to_owned(),
        image: "registry.example/workspace:1".to_owned(),
        resources: Resources {
            cpu_millis: 2_000,
            memory_mib: 4_096,
            gpu_count: 0,
            disk_gib: 50,
        },
        access_mode: AccessMode::Public,
        state,
        generation: 1,
    }
}

#[test]
fn workspace_generation_changes_the_pod_template_for_restart() {
    let mut first = workspace(WorkspaceState::Ready);
    first.generation = 7;
    let mut restarted = first.clone();
    restarted.generation = 8;
    restarted.state = WorkspaceState::Restarting;

    let first_template = builder()
        .build(&first)
        .unwrap()
        .stateful_set
        .spec
        .unwrap()
        .template;
    let restarted_template = builder()
        .build(&restarted)
        .unwrap()
        .stateful_set
        .spec
        .unwrap()
        .template;

    assert_ne!(first_template.metadata, restarted_template.metadata);
    assert_eq!(
        restarted_template.metadata.unwrap().annotations.unwrap()["workspace.memeloop.dev/generation"],
        "8"
    );
}

#[test]
fn internal_workspace_allows_cluster_ssh_without_a_public_jump_host() {
    let mut workspace = workspace(WorkspaceState::Ready);
    workspace.access_mode = AccessMode::Internal;
    let mut internal_builder = builder();
    internal_builder.web_shell_domain = None;
    let resources = internal_builder.build(&workspace).unwrap();
    assert!(resources.web_shell_route.is_none());
    let ingress = resources.network_policy.spec.unwrap().ingress.unwrap();
    let ssh = ingress
        .iter()
        .find(|rule| {
            rule.ports.as_ref().is_some_and(|ports| {
                ports.iter().any(|port| {
                    port.port
                        == Some(
                            k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(2222),
                        )
                })
            })
        })
        .unwrap();
    let peer = &ssh.from.as_ref().unwrap()[0];
    assert!(peer.namespace_selector.is_some());
    assert!(peer.pod_selector.is_none());
}

#[test]
fn builds_isolated_single_replica_workspace_with_standard_components() {
    let workspace = workspace(WorkspaceState::Ready);
    let resources = builder().build(&workspace).unwrap();
    assert_eq!(
        resources.namespace.metadata.name.as_deref(),
        Some("ws-public-a-01jabc")
    );
    assert_eq!(
        resources.stateful_set.spec.as_ref().unwrap().replicas,
        Some(1)
    );
    let containers = &resources
        .stateful_set
        .spec
        .as_ref()
        .unwrap()
        .template
        .spec
        .as_ref()
        .unwrap()
        .containers;
    assert_eq!(containers[0].name, "workspace");
    assert_eq!(containers[1].name, "ttyd");
    assert_eq!(
        containers[1].command.as_deref(),
        Some(["ttyd".to_owned()].as_slice())
    );
    assert!(
        containers[1]
            .args
            .as_ref()
            .unwrap()
            .contains(&"ssh".to_owned())
    );
    assert_eq!(
        containers[1].volume_mounts.as_ref().unwrap()[0].name,
        "runtime-ssh"
    );
    assert!(
        containers[0]
            .volume_mounts
            .as_ref()
            .unwrap()
            .iter()
            .any(|mount| mount.name == "runtime-ssh")
    );
    let pod_spec = resources
        .stateful_set
        .spec
        .as_ref()
        .unwrap()
        .template
        .spec
        .as_ref()
        .unwrap();
    assert_eq!(
        pod_spec.init_containers.as_ref().unwrap()[0]
            .command
            .as_ref()
            .unwrap()[0],
        "/usr/local/bin/mwc-workspace-bootstrap"
    );
    assert_eq!(
        resources.workspace_config.data.as_ref().unwrap()["sshd_config"]
            .lines()
            .find(|line| line.starts_with("HostKey")),
        Some("HostKey /etc/ssh/platform/ssh_host_ed25519_key")
    );
    assert_eq!(
        resources.service.spec.as_ref().unwrap().type_.as_deref(),
        Some("ClusterIP")
    );
    let route = resources.web_shell_route.as_ref().unwrap();
    assert_eq!(route.data["spec"]["hostnames"][0], "shell.example.com");
    assert_eq!(
        route.data["spec"]["rules"][0]["backendRefs"][0]["port"],
        7681
    );
    assert_eq!(
        route.data["spec"]["rules"][0]["matches"][0]["path"]["value"],
        "/shell/01jabc/"
    );
    let ingress = resources.network_policy.spec.unwrap().ingress.unwrap();
    assert!(ingress.iter().all(|rule| {
        let peer = &rule.from.as_ref().unwrap()[0];
        peer.namespace_selector.is_some() && peer.pod_selector.is_some()
    }));
}

#[test]
fn gpu_workspaces_request_the_standard_extended_resource() {
    let builder = builder();
    let mut workspace = workspace(WorkspaceState::Ready);
    workspace.resources.gpu_count = 2;

    let resources = builder
        .build(&workspace)
        .unwrap()
        .stateful_set
        .spec
        .unwrap()
        .template
        .spec
        .unwrap()
        .containers
        .into_iter()
        .find(|container| container.name == "workspace")
        .unwrap()
        .resources
        .unwrap();

    assert_eq!(resources.requests.unwrap()["nvidia.com/gpu"].0, "2");
    assert_eq!(resources.limits.unwrap()["nvidia.com/gpu"].0, "2");
}

#[test]
fn stopped_workspace_keeps_resources_and_scales_to_zero() {
    let resources = builder()
        .build(&workspace(WorkspaceState::Stopped))
        .unwrap();
    assert_eq!(resources.stateful_set.spec.unwrap().replicas, Some(0));
}

#[test]
fn delete_guard_requires_both_installation_and_workspace_labels() {
    let workspace = workspace(WorkspaceState::Ready);
    let resources = builder().build(&workspace).unwrap();
    builder()
        .verify_delete_ownership(&resources.namespace.metadata, workspace.id)
        .unwrap();
    assert!(matches!(
        ResourceBuilder {
            installation_id: "other".parse().unwrap(),
            ..builder()
        }
        .verify_delete_ownership(&resources.namespace.metadata, workspace.id),
        Err(OwnershipError::LabelMismatch {
            key: OWNER_INSTALLATION_LABEL,
            ..
        })
    ));
}

#[test]
fn deleting_workspace_has_no_desired_runtime_set() {
    assert!(matches!(
        builder().build(&workspace(WorkspaceState::Deleting)),
        Err(BuildError::WorkspaceBeingDeleted)
    ));
}

fn injection(
    key: &str,
    kind: InjectionKind,
    target: &str,
    value: &str,
    sensitive: bool,
) -> InjectionItem {
    InjectionItem {
        key: key.to_owned(),
        kind,
        target: target.to_owned(),
        value: InjectionValue::Utf8(value.to_owned()),
        sensitive,
        locked: false,
        version: 1,
        file_mode: Some(0o600),
        owner: Some("workspace".to_owned()),
        group: Some("workspace".to_owned()),
        template_selector: None,
        labels: BTreeMap::new(),
    }
}

#[test]
fn materializes_values_without_putting_secrets_in_metadata_or_manifest() {
    let secret_value = "token-secret-value";
    let config_value = "first\n\n  indented\nlast\n";
    let resolved = resolve_injections(
        &[injection(
            "token",
            InjectionKind::EnvironmentVariable,
            "REGISTRY_TOKEN",
            secret_value,
            true,
        )],
        &[],
        &[injection(
            "settings",
            InjectionKind::ConfigFile,
            "/workspace/.config/settings.yml",
            config_value,
            false,
        )],
    )
    .unwrap();
    let workspace = workspace(WorkspaceState::Ready);
    let materialized = builder()
        .materialize_injections(workspace.id, &workspace.short_id, &resolved)
        .unwrap();

    assert_eq!(
        materialized.environment_secret.data.as_ref().unwrap()["REGISTRY_TOKEN"].0,
        secret_value.as_bytes()
    );
    assert!(
        materialized
            .environment_config_map
            .data
            .as_ref()
            .unwrap()
            .is_empty()
    );
    assert!(
        materialized
            .file_config_map
            .data
            .as_ref()
            .unwrap()
            .iter()
            .any(|(key, value)| key.starts_with("file-") && value == config_value)
    );
    let manifest = &materialized.file_config_map.data.as_ref().unwrap()["workspace-files.json"];
    assert!(manifest.contains("/workspace/.config/settings.yml"));
    assert!(!manifest.contains(config_value));
    let serialized = serde_json::to_string(&materialized.environment_secret.metadata).unwrap();
    assert!(!serialized.contains(secret_value));
}
