use std::collections::BTreeSet;

use super::*;
use crate::{quota::Resources, workspace_runtime::WorkspaceResourceNames, workspaces::AccessMode};

fn resource_names() -> WorkspaceResourceNames {
    WorkspaceResourceNames::for_prefix("w-test")
}

fn template() -> WorkspaceTemplateSpec {
    WorkspaceTemplateSpec::standard(
        "registry.example/workspace:1",
        AccessMode::Internal,
        Resources {
            cpu_millis: 1_000,
            memory_mib: 1_024,
            gpu_count: 0,
            disk_gib: 10,
        },
    )
}

#[test]
fn injected_targets_do_not_remove_canonical_platform_environment() {
    let template = template();
    let pod = WorkspacePod::from_template(&template);
    let mut container = pod.workspace_container(
        "registry.example/workspace:1",
        ResourceRequirements::default(),
        &resource_names(),
    );
    apply_injected_environment_overrides(
        &template,
        &mut container,
        &BTreeSet::from(["HOME".to_owned(), "MWC_WORKSPACE_HOME".to_owned()]),
    );

    let environment = container.env.unwrap();
    assert_eq!(
        environment
            .iter()
            .filter(|item| item.name == "HOME")
            .map(|item| item.value.as_deref())
            .collect::<Vec<_>>(),
        [Some("/workspace")]
    );
    assert_eq!(
        environment
            .iter()
            .filter(|item| item.name == "MWC_WORKSPACE_HOME")
            .map(|item| item.value.as_deref())
            .collect::<Vec<_>>(),
        [Some("/workspace")]
    );
}

#[test]
fn sshd_set_env_quotes_spaces_and_quotation_marks() {
    let mut template = template();
    template.workspace_home = "/home/node-dev".to_owned();
    template.cluster_access = true;
    template.buildkit = true;
    let config = WorkspacePod::from_template(&template).ssh_set_env();
    assert!(config.contains("\"HOME=/home/node-dev\""));
    assert!(config.contains("\"RUSTUP_HOME=/usr/local/rustup\""));
    assert!(config.contains("\"KUBECONFIG=/run/mwc-ssh/kubeconfig\""));
    assert!(config.contains("\"BUILDKIT_HOST=tcp://127.0.0.1:1234\""));
    assert!(config.contains(
        "\"PATH=/run/mwc-buildkit/bin:/home/node-dev/.local/bin:/home/node-dev/.local/share/pnpm:/home/node-dev/.cargo/bin:/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin\""
    ));
    assert!(!config.contains("CARGO_HOME="));
}

#[test]
fn workspace_init_container_reuses_workspace_resource_requirements() {
    let template = template();
    let pod = WorkspacePod::from_template(&template);
    let resources = ResourceRequirements {
        requests: Some(pod.resource_requests()),
        limits: Some(pod.resource_limits()),
        ..ResourceRequirements::default()
    };
    let names = resource_names();
    let workspace = pod.workspace_container(&template.image, resources.clone(), &names);
    let init = pod.workspace_init_container(&template.image, resources, &names);

    assert_eq!(init.resources, workspace.resources);
    let init_resources = init.resources.expect("init container resources");
    assert_eq!(
        init_resources
            .requests
            .as_ref()
            .and_then(|values| values.get("ephemeral-storage")),
        Some(&Quantity("512Mi".to_owned()))
    );
    assert_eq!(
        init_resources
            .limits
            .as_ref()
            .and_then(|values| values.get("ephemeral-storage")),
        Some(&Quantity("2048Mi".to_owned()))
    );
}

#[test]
fn scratch_capacity_does_not_inflate_local_ephemeral_storage_limits() {
    let mut template = template();
    template.storage_policy.temporary_storage_gib = 256;

    let limits = WorkspacePod::from_template(&template).resource_limits();
    assert_eq!(
        limits.get("ephemeral-storage"),
        Some(&Quantity("2048Mi".to_owned()))
    );
}

#[test]
fn scratch_init_prepares_only_enabled_top_level_subpaths() {
    let mut template = template();
    template.buildkit = false;
    let container =
        WorkspacePod::from_template(&template).workspace_scratch_init_container(&template.image);

    assert_eq!(container.name, "workspace-scratch-init");
    let mount = &container.volume_mounts.as_ref().unwrap()[0];
    assert_eq!(mount.name, "workspace-scratch");
    assert_eq!(mount.mount_path, "/var/lib/mwc/workspace-scratch");
    assert!(mount.sub_path.is_none());
    let environment = container.env.as_ref().unwrap();
    assert!(environment.iter().any(|variable| {
        variable.name == "MWC_BUILDKIT_ENABLED" && variable.value.as_deref() == Some("false")
    }));
    let script = &container.args.as_ref().unwrap()[0];
    assert!(script.contains("workspace-cache"));
    assert!(script.contains("codex-session-scratch"));
    assert!(script.contains("if [ \"$MWC_BUILDKIT_ENABLED\" = true ]"));
    assert!(script.contains("prepare_directory \"$scratch_root/build-cache\" 1000 1000"));
    assert!(!script.contains("chown -R"));
}

#[test]
fn buildkit_disabled_omits_its_scratch_mount() {
    let mut template = template();
    template.buildkit = false;
    let mounts = WorkspacePod::from_template(&template)
        .workspace_container(
            &template.image,
            ResourceRequirements::default(),
            &resource_names(),
        )
        .volume_mounts
        .unwrap();

    assert!(mounts.iter().any(|mount| {
        mount.name == "workspace-scratch"
            && mount.mount_path == "/var/lib/mwc/build-scratch"
            && mount.sub_path.as_deref() == Some("workspace-cache")
    }));
    assert!(mounts.iter().any(|mount| {
        mount.name == "workspace-scratch"
            && mount.mount_path == "/var/lib/mwc/codex-scratch"
            && mount.sub_path.as_deref() == Some("codex-session-scratch")
    }));
    assert!(!mounts.iter().any(|mount| {
        mount.mount_path == "/run/mwc-buildkit"
            || mount
                .sub_path
                .as_deref()
                .is_some_and(|path| path.starts_with("build-cache"))
    }));
}

#[test]
fn home_disk_margin_uses_the_fixed_platform_algorithm() {
    assert_eq!(fixed_home_disk_margin_mib(5), 512);
    assert_eq!(fixed_home_disk_margin_mib(60), 1_024);
}

#[test]
fn desktop_template_injects_immutable_desktop_contract_and_declares_its_port() {
    let mut template = template();
    template.desktop = Some(crate::templates::DesktopEndpoint {
        internal_port: 6080,
        display_name: Some("Kali browser desktop".to_owned()),
    });
    let pod = WorkspacePod::from_template(&template);
    let mut container = pod.workspace_container(
        &template.image,
        ResourceRequirements::default(),
        &resource_names(),
    );
    apply_injected_environment_overrides(
        &template,
        &mut container,
        &BTreeSet::from([
            "MWC_DESKTOP_ENABLED".to_owned(),
            "MWC_DESKTOP_PORT".to_owned(),
        ]),
    );

    let environment = container.env.expect("workspace environment");
    assert!(environment.iter().any(|item| {
        item.name == "MWC_DESKTOP_ENABLED" && item.value.as_deref() == Some("true")
    }));
    assert!(
        environment.iter().any(|item| {
            item.name == "MWC_DESKTOP_PORT" && item.value.as_deref() == Some("6080")
        })
    );
    let ports = container.ports.expect("workspace ports");
    assert!(
        ports
            .iter()
            .any(|port| { port.name.as_deref() == Some("desktop") && port.container_port == 6080 })
    );
    let readiness = container
        .readiness_probe
        .and_then(|probe| probe.exec)
        .and_then(|action| action.command)
        .expect("desktop readiness command")
        .join(" ");
    assert!(readiness.contains("/run/mwc-ssh/sshd.pid"));
    assert!(readiness.contains("/run/mwc-ssh/desktop.pid"));
}
