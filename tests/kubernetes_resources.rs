use memeloop_workspace_control::{
    injections::{InjectionItem, InjectionKind, InjectionValue, resolve_injections},
    kubernetes::{
        BuildError, ORGANIZATION_ID_LABEL, OWNER_INSTALLATION_LABEL, OWNER_USER_ID_LABEL,
        OwnershipError, ResourceBuilder,
    },
    quota::Resources,
    templates::WorkspaceTemplateSpec,
    workspaces::{AccessMode, Workspace, WorkspaceState},
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
        higress_source_cidrs: vec!["100.64.0.6/31".to_owned()],
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

fn workspace(state: WorkspaceState) -> Workspace {
    Workspace {
        id: Uuid::now_v7(),
        short_id: "01jabc".to_owned(),
        organization_id: Uuid::now_v7(),
        owner_id: Uuid::now_v7(),
        name: "test-workspace".to_owned(),
        template_id: Some(Uuid::now_v7()),
        template: WorkspaceTemplateSpec::standard(
            "registry.example/workspace:1",
            AccessMode::Public,
            Resources {
                cpu_millis: 2_000,
                memory_mib: 4_096,
                gpu_count: 0,
                disk_gib: 50,
            },
        ),
        state,
        generation: 1,
        created_at: 1,
        updated_at: 1,
    }
}

fn node_template(image: &str, resources: Resources) -> WorkspaceTemplateSpec {
    let mut template = WorkspaceTemplateSpec::standard(image, AccessMode::Public, resources);
    template.workspace_user = "node-dev".to_owned();
    template.workspace_home = "/home/node-dev".to_owned();
    template.preserve_home_ownership = true;
    template.buildkit = true;
    template.pod_requests.cpu_millis = 1_000;
    template.pod_requests.memory_mib = 1_024;
    template.pod_requests.ephemeral_storage_mib = Some(256);
    template.ephemeral_storage_limit_mib = Some(1_024);
    template.required_node_names = vec!["westlake".to_owned(), "haixia".to_owned()];
    template.environment.extend([
        ("HOME".to_owned(), "/home/node-dev".to_owned()),
        (
            "PATH".to_owned(),
            "/usr/local/bin:/usr/local/sbin:/home/node-dev/.local/bin:/home/node-dev/.local/share/pnpm:/usr/sbin:/usr/bin:/sbin:/bin".to_owned(),
        ),
        (
            "BUILDKIT_HOST".to_owned(),
            "unix:///home/node-dev/.cache/buildkit/runtime/buildkit/buildkitd.sock".to_owned(),
        ),
    ]);
    template
}

fn rust_template(image: &str, resources: Resources) -> WorkspaceTemplateSpec {
    let mut template = WorkspaceTemplateSpec::standard(image, AccessMode::Public, resources);
    template.workspace_user = "rust-dev".to_owned();
    template.workspace_home = "/home/rust-dev".to_owned();
    template.preserve_home_ownership = true;
    template.buildkit = true;
    template.pod_requests.cpu_millis = 2_000;
    template.pod_requests.memory_mib = 4_096;
    template
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
fn observability_labels_do_not_change_statefulset_immutable_fields() {
    let workspace = workspace(WorkspaceState::Ready);
    let stateful_set = builder().build(&workspace).unwrap().stateful_set;
    let spec = stateful_set.spec.unwrap();
    let selector = spec.selector.match_labels.unwrap();
    let pod_labels = spec.template.metadata.unwrap().labels.unwrap();
    let volume_claim_templates = spec.volume_claim_templates.unwrap();
    let claim_labels = volume_claim_templates[0].metadata.labels.as_ref().unwrap();

    assert!(!selector.contains_key(ORGANIZATION_ID_LABEL));
    assert!(!selector.contains_key(OWNER_USER_ID_LABEL));
    assert_eq!(
        pod_labels[ORGANIZATION_ID_LABEL],
        workspace.organization_id.to_string()
    );
    assert_eq!(
        pod_labels[OWNER_USER_ID_LABEL],
        workspace.owner_id.to_string()
    );
    assert!(!claim_labels.contains_key(ORGANIZATION_ID_LABEL));
    assert!(!claim_labels.contains_key(OWNER_USER_ID_LABEL));
}

#[test]
fn internal_workspace_allows_cluster_ssh_without_a_public_jump_host() {
    let mut workspace = workspace(WorkspaceState::Ready);
    workspace.template.access_mode = AccessMode::Internal;
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
fn web_shell_allows_configured_host_network_gateway_sources() {
    let resources = builder().build(&workspace(WorkspaceState::Ready)).unwrap();
    let ingress = resources.network_policy.spec.unwrap().ingress.unwrap();
    let ttyd = ingress
        .iter()
        .find(|rule| {
            rule.ports.as_ref().is_some_and(|ports| {
                ports.iter().any(|port| {
                    port.port
                        == Some(
                            k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(7681),
                        )
                })
            })
        })
        .unwrap();
    assert!(ttyd.from.as_ref().unwrap().iter().any(|peer| {
        peer.ip_block
            .as_ref()
            .is_some_and(|block| block.cidr == "100.64.0.6/31")
    }));
}

#[test]
fn configured_tailnet_access_adds_an_ssh_only_automatic_node_port() {
    let mut workspace = workspace(WorkspaceState::Ready);
    workspace.template.access_mode = AccessMode::Internal;
    let mut tailnet_builder = builder();
    tailnet_builder.internal_ssh_node_port_enabled = true;
    let resources = tailnet_builder.build(&workspace).unwrap();

    assert_eq!(
        resources.service.spec.as_ref().unwrap().type_.as_deref(),
        Some("ClusterIP")
    );
    assert_eq!(
        resources
            .service
            .spec
            .as_ref()
            .unwrap()
            .publish_not_ready_addresses,
        Some(true)
    );
    let ssh_service = resources.internal_ssh_service.unwrap();
    let spec = ssh_service.spec.unwrap();
    assert_eq!(spec.type_.as_deref(), Some("NodePort"));
    assert_eq!(spec.publish_not_ready_addresses, Some(true));
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
fn only_templates_requesting_cluster_access_receive_an_owned_cluster_admin_identity() {
    let mut workspace = workspace(WorkspaceState::Ready);
    workspace.template.cluster_access = true;
    workspace.template.workspace_user = "cluster-admin".to_owned();
    workspace.template.workspace_home = "/home/cluster-admin".to_owned();
    workspace.template.environment.insert(
        "KUBECONFIG".to_owned(),
        "/home/cluster-admin/.mwc/kubeconfig".to_owned(),
    );
    let workspace_id = workspace.id;
    let resources = builder().build(&workspace).unwrap();

    let service_account = resources.service_account.unwrap();
    assert_eq!(
        service_account.metadata.name.as_deref(),
        Some("workspace-admin")
    );
    assert_eq!(
        service_account.metadata.namespace.as_deref(),
        Some("ws-public-a-01jabc")
    );
    assert_eq!(service_account.automount_service_account_token, Some(true));

    let binding = resources.cluster_role_binding.unwrap();
    assert_eq!(
        binding.metadata.name.as_deref(),
        Some("mwc-public-a-01jabc-admin")
    );
    assert_eq!(binding.role_ref.kind, "ClusterRole");
    assert_eq!(binding.role_ref.name, "cluster-admin");
    let subject = &binding.subjects.unwrap()[0];
    assert_eq!(subject.kind, "ServiceAccount");
    assert_eq!(subject.name, "workspace-admin");
    assert_eq!(subject.namespace.as_deref(), Some("ws-public-a-01jabc"));
    builder()
        .verify_delete_ownership(&binding.metadata, workspace_id)
        .unwrap();
    assert!(matches!(
        ResourceBuilder {
            installation_id: "other".parse().unwrap(),
            ..builder()
        }
        .verify_delete_ownership(&binding.metadata, workspace_id),
        Err(OwnershipError::LabelMismatch {
            key: OWNER_INSTALLATION_LABEL,
            ..
        })
    ));

    let pod = resources.stateful_set.spec.unwrap().template.spec.unwrap();
    assert_eq!(pod.service_account_name.as_deref(), Some("workspace-admin"));
    assert_eq!(pod.automount_service_account_token, Some(true));
    let workspace_container = pod
        .containers
        .iter()
        .find(|container| container.name == "workspace")
        .unwrap();
    let environment = workspace_container.env.as_ref().unwrap();
    assert!(environment.iter().any(|variable| {
        variable.name == "MWC_IN_CLUSTER_KUBECONFIG" && variable.value.as_deref() == Some("true")
    }));
    assert!(environment.iter().any(|variable| {
        variable.name == "KUBECONFIG"
            && variable.value.as_deref() == Some("/run/mwc-ssh/kubeconfig")
    }));
    let config = resources.workspace_config.data.as_ref().unwrap();
    assert!(config["sshd_config"].contains("\"KUBECONFIG=/run/mwc-ssh/kubeconfig\""));
    assert!(config["mwc-workspace-bootstrap"].contains("server: https://kubernetes.default.svc"));
    assert!(
        config["mwc-workspace-bootstrap"]
            .contains("tokenFile: /var/run/secrets/kubernetes.io/serviceaccount/token")
    );
}

#[test]
fn templates_without_cluster_access_never_receive_a_service_account_token() {
    for (name, template) in [
        (
            "standard",
            WorkspaceTemplateSpec::standard(
                "registry.example/workspace:1",
                AccessMode::Public,
                Resources {
                    cpu_millis: 2_000,
                    memory_mib: 4_096,
                    gpu_count: 0,
                    disk_gib: 50,
                },
            ),
        ),
        (
            "rust",
            rust_template(
                "registry.example/workspace:1",
                Resources {
                    cpu_millis: 6_000,
                    memory_mib: 8_192,
                    gpu_count: 0,
                    disk_gib: 50,
                },
            ),
        ),
        (
            "node",
            node_template(
                "registry.example/workspace:1",
                Resources {
                    cpu_millis: 6_000,
                    memory_mib: 4_096,
                    gpu_count: 0,
                    disk_gib: 50,
                },
            ),
        ),
    ] {
        let mut workspace = workspace(WorkspaceState::Ready);
        workspace.template = template;
        let resources = builder().build(&workspace).unwrap();
        assert!(resources.service_account.is_none(), "{name}");
        assert!(resources.cluster_role_binding.is_none(), "{name}");
        let pod = resources.stateful_set.spec.unwrap().template.spec.unwrap();
        assert_eq!(pod.service_account_name, None, "{name}");
        assert_eq!(pod.automount_service_account_token, Some(false), "{name}");
        let workspace_container = pod
            .containers
            .iter()
            .find(|container| container.name == "workspace")
            .unwrap();
        assert!(
            workspace_container
                .env
                .as_ref()
                .unwrap()
                .iter()
                .any(|variable| {
                    variable.name == "MWC_IN_CLUSTER_KUBECONFIG"
                        && variable.value.as_deref() == Some("false")
                })
        );
        assert!(
            !workspace_container
                .env
                .as_ref()
                .unwrap()
                .iter()
                .any(|variable| variable.name == "KUBECONFIG")
        );
    }
}

#[test]
fn builds_isolated_single_replica_workspace_with_standard_components() {
    let workspace = workspace(WorkspaceState::Ready);
    let resources = builder().build(&workspace).unwrap();
    assert_eq!(
        resources.namespace.metadata.name.as_deref(),
        Some("ws-public-a-01jabc")
    );
    let namespace_labels = resources.namespace.metadata.labels.as_ref().unwrap();
    assert_eq!(
        namespace_labels["workspace.memeloop.dev/organization-id"],
        workspace.organization_id.to_string()
    );
    assert_eq!(
        namespace_labels["workspace.memeloop.dev/owner-user-id"],
        workspace.owner_id.to_string()
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
    let ttyd_resources = containers[1].resources.as_ref().unwrap();
    assert_eq!(ttyd_resources.requests.as_ref().unwrap()["cpu"].0, "10m");
    assert_eq!(
        ttyd_resources.requests.as_ref().unwrap()["memory"].0,
        "16Mi"
    );
    assert_eq!(
        containers[1].command.as_deref(),
        Some(["/usr/bin/ttyd".to_owned()].as_slice())
    );
    assert!(
        containers[1]
            .args
            .as_ref()
            .unwrap()
            .contains(&"/usr/bin/ssh".to_owned())
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
    workspace.template.resources.gpu_count = 2;

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
fn node_template_reuses_the_existing_image_with_platform_bootstrap() {
    let mut workspace = workspace(WorkspaceState::Ready);
    workspace.template = node_template(
        "harbor.k3s.onetwo.website/library/node-dev:fixed@sha256:abc",
        Resources {
            cpu_millis: 6_000,
            memory_mib: 4_096,
            gpu_count: 0,
            disk_gib: 30,
        },
    );

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
    assert_eq!(
        dev.image.as_deref(),
        Some(workspace.template.image.as_str())
    );
    assert_eq!(
        dev.command.as_deref(),
        Some(["/etc/workspace-platform/mwc-workspace-bootstrap".to_owned()].as_slice())
    );
    assert_eq!(dev.args.as_deref(), Some(["serve".to_owned()].as_slice()));
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
    assert_eq!(quantities.requests.as_ref().unwrap()["cpu"].0, "1000m");
    assert_eq!(quantities.requests.as_ref().unwrap()["memory"].0, "1024Mi");
    assert_eq!(quantities.limits.as_ref().unwrap()["cpu"].0, "6000m");
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
    assert_eq!(pod.init_containers.as_ref().unwrap().len(), 1);
    assert!(
        resources.workspace_config.data.as_ref().unwrap()["sshd_config"]
            .contains("AllowUsers node-dev")
    );
    assert!(
        resources.workspace_config.data.as_ref().unwrap()["sshd_config"].contains("StrictModes no")
    );
    let sshd_config = &resources.workspace_config.data.as_ref().unwrap()["sshd_config"];
    assert_eq!(sshd_config.matches("SetEnv ").count(), 1);
    assert!(sshd_config.contains(
        "\"PATH=/run/mwc-buildkit/bin:/usr/local/bin:/usr/local/sbin:/home/node-dev/.local/bin"
    ));
    assert!(
        sshd_config
            .contains("\"BUILDKIT_HOST=unix:///run/mwc-buildkit/runtime/buildkit/buildkitd.sock\"")
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
    assert!(
        resources.workspace_config.data.as_ref().unwrap()["mwc-workspace-bootstrap"]
            .contains("usermod -p '*' \"$workspace_user\"")
    );
    let bootstrap = &resources.workspace_config.data.as_ref().unwrap()["mwc-workspace-bootstrap"];
    assert!(bootstrap.contains(".owner // empty"));
    assert!(bootstrap.contains("owner=$workspace_user"));
    assert!(bootstrap.contains("secret) mode=384"));
    assert!(bootstrap.contains("config_map) mode=420"));
    assert!(bootstrap.contains("prepare_runtime_sshd_config"));
    assert!(bootstrap.contains("mark_home_degraded"));
    assert!(bootstrap.contains("regenerable_link_best_effort"));
    assert!(bootstrap.contains("release_reserve_if_critical"));
    assert!(bootstrap.contains("exec /usr/sbin/sshd -D -e -f \"$runtime_sshd_config\""));
}

#[test]
fn regenerable_data_uses_bounded_pod_lifetime_storage() {
    let mut workspace = workspace(WorkspaceState::Ready);
    workspace.template.buildkit = true;
    workspace.template.storage_policy.runtime_tmp_memory_mib = 640;
    workspace.template.storage_policy.build_scratch_gib = 14;
    workspace.template.storage_policy.buildkit_cache_gib = 9;
    workspace.template.storage_policy.codex_scratch_gib = 3;

    let resources = builder().build(&workspace).unwrap();
    let pod = resources.stateful_set.spec.unwrap().template.spec.unwrap();
    let volume = |name: &str| {
        pod.volumes
            .as_ref()
            .unwrap()
            .iter()
            .find(|volume| volume.name == name)
            .unwrap()
            .empty_dir
            .as_ref()
            .unwrap()
    };
    assert_eq!(volume("runtime-tmp").medium.as_deref(), Some("Memory"));
    assert_eq!(
        volume("runtime-tmp").size_limit.as_ref().unwrap().0,
        "640Mi"
    );
    assert_eq!(
        volume("build-scratch").size_limit.as_ref().unwrap().0,
        "14Gi"
    );
    assert_eq!(
        volume("buildkit-cache").size_limit.as_ref().unwrap().0,
        "9Gi"
    );
    assert_eq!(
        volume("codex-scratch").size_limit.as_ref().unwrap().0,
        "3Gi"
    );
    assert_eq!(
        volume("runtime-ssh").size_limit.as_ref().unwrap().0,
        "128Mi"
    );
    assert_eq!(volume("runtime-ssh").medium.as_deref(), Some("Memory"));
    assert_eq!(volume("ttyd-tmp").medium.as_deref(), Some("Memory"));
    assert_eq!(volume("ttyd-tmp").size_limit.as_ref().unwrap().0, "128Mi");

    let workspace_container = pod
        .containers
        .iter()
        .find(|container| container.name == "workspace")
        .unwrap();
    let mounts = workspace_container.volume_mounts.as_ref().unwrap();
    assert!(mounts.iter().any(|mount| {
        mount.name == "runtime-tmp" && matches!(mount.mount_path.as_str(), "/tmp" | "/var/tmp")
    }));
    assert!(mounts.iter().any(|mount| {
        mount.name == "build-scratch" && mount.mount_path == "/var/lib/mwc/build-scratch"
    }));
    assert!(mounts.iter().any(|mount| {
        mount.name == "codex-scratch" && mount.mount_path == "/var/lib/mwc/codex-scratch"
    }));
    assert!(mounts.iter().any(|mount| {
        mount.name == "buildkit-cache" && mount.mount_path == "/run/mwc-buildkit"
    }));
    assert!(!mounts.iter().any(|mount| {
        mount.name == "workspace-data" && matches!(mount.mount_path.as_str(), "/tmp" | "/var/tmp")
    }));
    let ttyd = pod
        .containers
        .iter()
        .find(|container| container.name == "ttyd")
        .unwrap();
    let ttyd_mounts = ttyd.volume_mounts.as_ref().unwrap();
    assert!(ttyd_mounts.iter().any(|mount| {
        mount.name == "ttyd-tmp" && matches!(mount.mount_path.as_str(), "/tmp" | "/var/tmp")
    }));
    assert!(!ttyd_mounts.iter().any(|mount| mount.name == "runtime-tmp"));
    let environment = workspace_container.env.as_ref().unwrap();
    assert!(environment.iter().any(|variable| {
        variable.name == "CARGO_TARGET_DIR"
            && variable.value.as_deref() == Some("/var/lib/mwc/build-scratch/cargo-target")
    }));
    assert!(environment.iter().any(|variable| {
        variable.name == "BUILDKIT_HOST"
            && variable.value.as_deref()
                == Some("unix:///run/mwc-buildkit/runtime/buildkit/buildkitd.sock")
    }));
    assert!(environment.iter().any(|variable| {
        variable.name == "MWC_HOME_RESERVE_MIB" && variable.value.as_deref() == Some("1024")
    }));
    assert_eq!(
        workspace_container
            .resources
            .as_ref()
            .unwrap()
            .limits
            .as_ref()
            .unwrap()["ephemeral-storage"]
            .0,
        "17664Mi"
    );

    let buildkit = pod
        .containers
        .iter()
        .find(|container| container.name == "buildkitd")
        .unwrap();
    assert!(
        buildkit
            .volume_mounts
            .as_ref()
            .unwrap()
            .iter()
            .any(|mount| {
                mount.name == "buildkit-cache" && mount.mount_path == "/var/lib/mwc-buildkit"
            })
    );
    assert!(
        !buildkit
            .volume_mounts
            .as_ref()
            .unwrap()
            .iter()
            .any(|mount| {
                mount.name == "workspace-data"
                    && matches!(mount.mount_path.as_str(), "/tmp" | "/var/tmp")
            })
    );
    assert_eq!(
        buildkit
            .resources
            .as_ref()
            .unwrap()
            .limits
            .as_ref()
            .unwrap()["ephemeral-storage"]
            .0,
        "9Gi"
    );
    assert_eq!(
        buildkit
            .resources
            .as_ref()
            .unwrap()
            .requests
            .as_ref()
            .unwrap()["ephemeral-storage"]
            .0,
        "1Gi"
    );

    let sshd = &resources.workspace_config.data.as_ref().unwrap()["sshd_config"];
    assert!(sshd.contains("AuthorizedKeysFile /run/mwc-ssh/authorized_keys"));
    assert!(sshd.contains("Banner /run/mwc-ssh/storage-banner"));
}

#[test]
fn rust_template_uses_the_single_canonical_rust_home() {
    let mut workspace = workspace(WorkspaceState::Ready);
    workspace.template = rust_template(
        "registry.example/workspace:1",
        Resources {
            cpu_millis: 6_000,
            memory_mib: 8_192,
            gpu_count: 0,
            disk_gib: 50,
        },
    );
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
    assert!(
        mounts.iter().any(|mount| {
            mount.name == "workspace-data" && mount.mount_path == "/home/rust-dev"
        })
    );
    assert!(!mounts.iter().any(|mount| {
        mount.name == "workspace-data" && mount.mount_path == "/home/token-center-dev"
    }));
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
    let environment_manifest =
        &materialized.file_config_map.data.as_ref().unwrap()["workspace-environment.json"];
    assert!(environment_manifest.contains("REGISTRY_TOKEN"));
    assert!(environment_manifest.contains("env-0000"));
    assert!(!environment_manifest.contains(secret_value));
    assert_eq!(
        materialized.file_secret.data.as_ref().unwrap()["env-0000"].0,
        secret_value.as_bytes()
    );
    let serialized = serde_json::to_string(&materialized.environment_secret.metadata).unwrap();
    assert!(!serialized.contains(secret_value));
}
