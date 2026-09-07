use std::collections::BTreeSet;

use super::*;
use crate::{quota::Resources, workspace_runtime::WorkspaceResourceNames, workspaces::AccessMode};

fn resource_names() -> WorkspaceResourceNames {
    WorkspaceResourceNames::for_prefix("w-test")
}

fn template() -> WorkspaceTemplateSpec {
    let template = WorkspaceTemplateSpec::standard(
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
    assert!(
        config
            .contains("\"BUILDKIT_HOST=unix:///run/mwc-buildkit/runtime/buildkit/buildkitd.sock\"")
    );
    assert!(config.contains(
        "\"PATH=/run/mwc-buildkit/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin\""
    ));
}
