use std::collections::BTreeSet;

use super::*;
use crate::{quota::Resources, workspace_runtime::WorkspaceResourceNames, workspaces::AccessMode};

fn resource_names() -> WorkspaceResourceNames {
    WorkspaceResourceNames::for_prefix("w-test")
}

fn template() -> WorkspaceTemplateSpec {
    let mut template = WorkspaceTemplateSpec::standard(
        "registry.example/workspace:1",
        AccessMode::Internal,
        Resources {
            cpu_millis: 1_000,
            memory_mib: 1_024,
            gpu_count: 0,
            disk_gib: 10,
        },
    );
    template
        .environment
        .insert("HOME".to_owned(), "/previous home".to_owned());
    template.environment.insert(
        "MWC_WORKSPACE_HOME".to_owned(),
        "/must-not-shadow-platform".to_owned(),
    );
    template
        .environment
        .insert("INJECTED_TOKEN".to_owned(), "previous".to_owned());
    template
}

#[test]
fn injected_targets_remove_only_previous_template_environment() {
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
        &BTreeSet::from([
            "HOME".to_owned(),
            "INJECTED_TOKEN".to_owned(),
            "MWC_WORKSPACE_HOME".to_owned(),
        ]),
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
    assert!(environment.iter().all(|item| item.name != "INJECTED_TOKEN"));
}

#[test]
fn sshd_set_env_quotes_spaces_and_quotation_marks() {
    let mut template = template();
    template.workspace_home = "/home/node-dev".to_owned();
    template.cluster_access = true;
    template.buildkit = true;
    template
        .environment
        .insert("TOOL_FLAGS".to_owned(), "--name \"hello world\"".to_owned());
    let config = WorkspacePod::from_template(&template).ssh_set_env();
    assert!(config.contains("\"HOME=/home/node-dev\""));
    assert!(config.contains("\"KUBECONFIG=/run/mwc-ssh/kubeconfig\""));
    assert!(
        config
            .contains("\"BUILDKIT_HOST=unix:///run/mwc-buildkit/runtime/buildkit/buildkitd.sock\"")
    );
    assert!(config.contains("\"TOOL_FLAGS=--name \\\"hello world\\\"\""));
    assert!(config.contains(
        "\"PATH=/home/node-dev/.local/bin:/home/node-dev/.local/share/pnpm:/home/node-dev/.cargo/bin:/usr/local/cargo/bin:/run/mwc-buildkit/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin\""
    ));
}

#[test]
fn sshd_set_env_sets_a_complete_path_for_non_buildkit_images() {
    let mut template = template();
    template.workspace_home = "/home/rust-dev".to_owned();
    template.environment.insert(
        "PATH".to_owned(),
        "/opt/workspace-tools:/usr/local/bin:/bin".to_owned(),
    );

    let config = WorkspacePod::from_template(&template).ssh_set_env();

    assert!(config.contains(
        "\"PATH=/home/rust-dev/.local/bin:/home/rust-dev/.local/share/pnpm:/home/rust-dev/.cargo/bin:/usr/local/cargo/bin:/run/mwc-buildkit/bin:/opt/workspace-tools:/usr/local/bin:/bin\""
    ));
}
