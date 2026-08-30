use std::collections::BTreeMap;

use k8s_openapi::{
    api::core::v1::{
        ConfigMapEnvSource, EnvFromSource, EnvVar, NodeSelectorRequirement, NodeSelectorTerm,
        SeccompProfile, SecretEnvSource, SecurityContext,
    },
    apimachinery::pkg::api::resource::Quantity,
};

pub(super) fn hostname_term(values: &[String]) -> NodeSelectorTerm {
    NodeSelectorTerm {
        match_expressions: Some(vec![NodeSelectorRequirement {
            key: "kubernetes.io/hostname".to_owned(),
            operator: "In".to_owned(),
            values: Some(values.to_vec()),
        }]),
        ..NodeSelectorTerm::default()
    }
}

pub(super) fn injection_env_from() -> Vec<EnvFromSource> {
    vec![
        EnvFromSource {
            config_map_ref: Some(ConfigMapEnvSource {
                name: "workspace-environment-config".to_owned(),
                optional: Some(false),
            }),
            ..EnvFromSource::default()
        },
        EnvFromSource {
            secret_ref: Some(SecretEnvSource {
                name: "workspace-environment-secret".to_owned(),
                optional: Some(false),
            }),
            ..EnvFromSource::default()
        },
    ]
}

pub(super) fn quantities(
    cpu: impl Into<String>,
    memory: impl Into<String>,
    ephemeral: Option<String>,
) -> BTreeMap<String, Quantity> {
    let mut values = BTreeMap::from([
        ("cpu".to_owned(), Quantity(cpu.into())),
        ("memory".to_owned(), Quantity(memory.into())),
    ]);
    if let Some(ephemeral) = ephemeral {
        values.insert("ephemeral-storage".to_owned(), Quantity(ephemeral));
    }
    values
}

pub(super) fn env(name: &str, value: &str) -> EnvVar {
    EnvVar {
        name: name.to_owned(),
        value: Some(value.to_owned()),
        ..EnvVar::default()
    }
}

pub(super) fn sshd_argument(name: &str, value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{name}={escaped}\"")
}

pub(super) fn root_security_context(read_only_root: bool) -> SecurityContext {
    SecurityContext {
        run_as_user: Some(0),
        run_as_group: Some(0),
        run_as_non_root: Some(false),
        allow_privilege_escalation: Some(false),
        read_only_root_filesystem: Some(read_only_root),
        // Both stock sshd and the explicit Debian compatibility path run as root. Keep the
        // runtime's normal root capability set: sshd must switch to the image's unprivileged
        // account, while apt/dpkg must create root-owned files. No host namespaces, privileged
        // flag, host mounts or service-account token are granted.
        capabilities: None,
        seccomp_profile: Some(SeccompProfile {
            type_: "RuntimeDefault".to_owned(),
            ..SeccompProfile::default()
        }),
        ..SecurityContext::default()
    }
}
