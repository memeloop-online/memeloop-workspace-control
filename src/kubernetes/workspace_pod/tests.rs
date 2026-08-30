use std::collections::BTreeSet;

use super::*;
use crate::{quota::Resources, workspaces::AccessMode};

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
        .insert("HOME".to_owned(), "/legacy home".to_owned());
    template.environment.insert(
        "MWC_WORKSPACE_HOME".to_owned(),
        "/must-not-shadow-platform".to_owned(),
    );
    template
        .environment
        .insert("LEGACY_TOKEN".to_owned(), "legacy".to_owned());
    template
}

#[test]
fn injected_targets_remove_only_legacy_template_environment() {
    let template = template();
    let pod = WorkspacePod::from_template(&template);
    let mut container = pod.workspace_container(
        "registry.example/workspace:1",
        ResourceRequirements::default(),
    );
    suppress_legacy_environment(
        &template,
        &mut container,
        &BTreeSet::from([
            "HOME".to_owned(),
            "LEGACY_TOKEN".to_owned(),
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
    assert!(environment.iter().all(|item| item.name != "LEGACY_TOKEN"));
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
    assert!(config.contains("\"PATH=/run/mwc-buildkit/bin:"));
}
