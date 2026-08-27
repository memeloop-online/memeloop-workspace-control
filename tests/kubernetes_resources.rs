use memeloop_workspace_control::{
    injections::{InjectionItem, InjectionKind, InjectionValue, resolve_injections},
    kubernetes::{
        BuildError, OWNER_INSTALLATION_LABEL, OwnershipError, ResourceBuilder,
        WorkspaceResourceSpec,
    },
    quota::Resources,
    workspaces::{AccessMode, WorkspaceRuntimeProfile, WorkspaceState},
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
        internal_ssh_node_port_enabled: false,
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
        runtime_profile: WorkspaceRuntimeProfile::Standard,
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
    assert!(resources.web_shell_ingress.is_none());
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
fn configured_tailnet_access_adds_an_ssh_only_automatic_node_port() {
    let mut workspace = workspace(WorkspaceState::Ready);
    workspace.access_mode = AccessMode::Internal;
    let mut tailnet_builder = builder();
    tailnet_builder.internal_ssh_node_port_enabled = true;
    let resources = tailnet_builder.build(&workspace).unwrap();

    assert_eq!(
        resources.service.spec.as_ref().unwrap().type_.as_deref(),
        Some("ClusterIP")
    );
    let ssh_service = resources.internal_ssh_service.unwrap();
    let spec = ssh_service.spec.unwrap();
    assert_eq!(spec.type_.as_deref(), Some("NodePort"));
    let ports = spec.ports.unwrap();
    assert_eq!(ports.len(), 1, "ttyd must never be published by NodePort");
    assert_eq!(ports[0].name.as_deref(), Some("ssh"));
    assert_eq!(ports[0].port, 2222);
    assert_eq!(ports[0].node_port, None, "Kubernetes assigns the port");

    let ssh_rule = resources
        .network_policy
        .spec
        .unwrap()
        .ingress
        .unwrap()
        .into_iter()
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
    assert!(ssh_rule.from.unwrap().iter().any(|peer| {
        peer.ip_block
            .as_ref()
            .is_some_and(|block| block.cidr == "100.64.0.0/10")
    }));
}

#[test]
fn tailnet_node_port_is_not_created_for_public_workspaces() {
    let workspace = workspace(WorkspaceState::Ready);
    let mut tailnet_builder = builder();
    tailnet_builder.internal_ssh_node_port_enabled = true;
    assert!(
        tailnet_builder
            .build(&workspace)
            .unwrap()
            .internal_ssh_service
            .is_none()
    );
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
    let ttyd_args = containers[1].args.as_ref().unwrap();
    let base_path_index = ttyd_args
        .iter()
        .position(|argument| argument == "--base-path")
        .unwrap();
    assert_eq!(ttyd_args[base_path_index + 1], "/shell/01jabc");
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
        "/etc/workspace-platform/mwc-workspace-bootstrap"
    );
    assert_eq!(
        resources.workspace_config.data.as_ref().unwrap()["sshd_config"]
            .lines()
            .find(|line| line.starts_with("HostKey")),
        Some("HostKey /run/mwc-ssh/ssh_host_ed25519_key")
    );
    assert!(
        resources.workspace_config.data.as_ref().unwrap()["sshd_config"]
            .contains("StrictModes yes")
    );
    assert_eq!(
        resources.service.spec.as_ref().unwrap().type_.as_deref(),
        Some("ClusterIP")
    );
    let web_shell_ingress = resources.web_shell_ingress.as_ref().unwrap();
    assert_eq!(
        web_shell_ingress.metadata.name.as_deref(),
        Some("web-shell")
    );
    assert_eq!(
        web_shell_ingress.metadata.namespace.as_deref(),
        Some("ws-public-a-01jabc")
    );
    let web_shell_labels = web_shell_ingress.metadata.labels.as_ref().unwrap();
    assert_eq!(web_shell_labels[OWNER_INSTALLATION_LABEL], "public-a");
    assert_eq!(
        web_shell_labels["workspace.memeloop.dev/workspace-id"],
        workspace.id.to_string()
    );
    let web_shell_spec = web_shell_ingress.spec.as_ref().unwrap();
    assert_eq!(web_shell_spec.ingress_class_name.as_deref(), Some("nginx"));
    let web_shell_rule = &web_shell_spec.rules.as_ref().unwrap()[0];
    assert_eq!(web_shell_rule.host.as_deref(), Some("shell.example.com"));
    let web_shell_path = &web_shell_rule.http.as_ref().unwrap().paths[0];
    assert_eq!(web_shell_path.path.as_deref(), Some("/shell/01jabc/"));
    assert_eq!(web_shell_path.path_type, "Prefix");
    let web_shell_backend = web_shell_path.backend.service.as_ref().unwrap();
    assert_eq!(web_shell_backend.name, "workspace");
    assert_eq!(web_shell_backend.port.as_ref().unwrap().number, Some(7681));
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
fn coder_node_profile_reuses_the_legacy_image_with_platform_bootstrap() {
    let mut workspace = workspace(WorkspaceState::Ready);
    workspace.runtime_profile = WorkspaceRuntimeProfile::CoderNodeDev;
    workspace.image = "harbor.k3s.onetwo.website/library/node-dev:fixed@sha256:abc".to_owned();
    workspace.resources.cpu_millis = 6_000;
    workspace.resources.memory_mib = 4_096;
    workspace.resources.disk_gib = 30;

    let resources = builder().build(&workspace).unwrap();
    let pod = resources
        .stateful_set
        .spec
        .as_ref()
        .unwrap()
        .template
        .spec
        .as_ref()
        .unwrap();
    let dev = pod
        .containers
        .iter()
        .find(|container| container.name == "workspace")
        .unwrap();
    assert_eq!(dev.image.as_deref(), Some(workspace.image.as_str()));
    assert_eq!(
        dev.command.as_deref(),
        Some(["/etc/workspace-platform/mwc-workspace-bootstrap".to_owned()].as_slice())
    );
    assert_eq!(
        dev.args.as_deref(),
        Some(["compat-serve".to_owned()].as_slice())
    );
    let readiness = dev.readiness_probe.as_ref().unwrap();
    assert_eq!(
        readiness.tcp_socket.as_ref().unwrap().port,
        k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(2222)
    );
    assert!(
        dev.volume_mounts.as_ref().unwrap().iter().any(|mount| {
            mount.name == "workspace-data" && mount.mount_path == "/home/node-dev"
        })
    );
    let quantities = dev.resources.as_ref().unwrap();
    assert_eq!(quantities.requests.as_ref().unwrap()["cpu"].0, "1");
    assert_eq!(quantities.requests.as_ref().unwrap()["memory"].0, "1Gi");
    assert_eq!(quantities.limits.as_ref().unwrap()["cpu"].0, "6");
    assert_eq!(quantities.limits.as_ref().unwrap()["memory"].0, "4096Mi");
    assert!(pod.containers.iter().any(|container| {
        container.name == "buildkitd"
            && container.image.as_deref().is_some_and(|image| {
                image.starts_with("harbor.k3s.onetwo.website/") && image.contains("@sha256:")
            })
    }));
    let buildkit = pod
        .containers
        .iter()
        .find(|container| container.name == "buildkitd")
        .unwrap();
    assert_eq!(
        buildkit
            .security_context
            .as_ref()
            .unwrap()
            .app_armor_profile
            .as_ref()
            .unwrap()
            .type_,
        "Unconfined"
    );
    assert!(
        pod.init_containers
            .as_ref()
            .unwrap()
            .iter()
            .any(|container| {
                container.name == "buildctl"
                    && container.image.as_deref().is_some_and(|image| {
                        image.starts_with("harbor.k3s.onetwo.website/")
                            && image.contains("@sha256:")
                    })
            })
    );
    assert!(
        resources.workspace_config.data.as_ref().unwrap()["sshd_config"]
            .contains("AllowUsers node-dev")
    );
    assert!(
        resources.workspace_config.data.as_ref().unwrap()["sshd_config"].contains("StrictModes no")
    );
    assert!(
        resources.workspace_config.data.as_ref().unwrap()["mwc-workspace-bootstrap"]
            .contains("apt-get install -y --no-install-recommends jq openssh-server")
    );
    assert!(
        resources.workspace_config.data.as_ref().unwrap()["mwc-workspace-bootstrap"]
            .contains("install -d -m 1777")
    );
    assert!(
        resources.workspace_config.data.as_ref().unwrap()["mwc-workspace-bootstrap"]
            .contains("$runtime_dir/ssh_host_ed25519_key")
    );
}

#[test]
fn token_center_profile_preserves_both_legacy_home_mounts() {
    let mut workspace = workspace(WorkspaceState::Ready);
    workspace.runtime_profile = WorkspaceRuntimeProfile::CoderTokenCenterRustDev;
    let resources = builder().build(&workspace).unwrap();
    let mounts = resources
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
        .volume_mounts
        .unwrap();
    assert!(mounts.iter().any(|mount| {
        mount.name == "workspace-data" && mount.mount_path == "/home/token-center-dev"
    }));
    assert!(
        mounts.iter().any(|mount| {
            mount.name == "workspace-data" && mount.mount_path == "/home/rust-dev"
        })
    );
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
