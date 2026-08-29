use std::collections::BTreeMap;

use k8s_openapi::{
    api::core::v1::{
        Affinity, ConfigMapEnvSource, Container, ContainerPort, EnvFromSource, EnvVar,
        LocalObjectReference, NodeAffinity, NodeSelector, NodeSelectorRequirement,
        NodeSelectorTerm, PodSecurityContext, PreferredSchedulingTerm, Probe, ResourceRequirements,
        SeccompProfile, SecretEnvSource, SecurityContext, TCPSocketAction, VolumeMount,
    },
    apimachinery::pkg::{api::resource::Quantity, util::intstr::IntOrString},
};

use crate::templates::WorkspaceTemplateSpec;

use super::{buildkit, resource_helpers::workspace_mounts};

const BOOTSTRAP: &str = "/etc/workspace-platform/mwc-workspace-bootstrap";
const INTERNAL_PLATFORM_ENVIRONMENT: [&str; 5] = [
    "MWC_WORKSPACE_USER",
    "MWC_WORKSPACE_HOME",
    "MWC_IN_CLUSTER_KUBECONFIG",
    "MWC_PRESERVE_HOME_ROOT",
    "MWC_BUILDKIT_ENABLED",
];
#[derive(Clone, Copy)]
pub(super) struct WorkspacePod<'a> {
    pub login_user: &'a str,
    pub home: &'a str,
    template: &'a WorkspaceTemplateSpec,
}

impl<'a> WorkspacePod<'a> {
    pub fn from_template(template: &'a WorkspaceTemplateSpec) -> Self {
        Self {
            login_user: &template.workspace_user,
            home: &template.workspace_home,
            template,
        }
    }

    pub fn resource_requests(&self) -> BTreeMap<String, Quantity> {
        quantities(
            format!("{}m", self.template.pod_requests.cpu_millis),
            format!("{}Mi", self.template.pod_requests.memory_mib),
            self.template
                .pod_requests
                .ephemeral_storage_mib
                .map(|value| format!("{value}Mi")),
        )
    }

    pub fn ssh_strict_modes(self) -> &'static str {
        if self.template.preserve_home_ownership {
            // Migrated Coder PVC roots are intentionally root:1000/2775. The platform owns the
            // generated authorized_keys file and keeps it 0600, so retaining the legacy root
            // metadata requires disabling only sshd's parent-directory ownership check.
            "no"
        } else {
            "yes"
        }
    }

    pub fn ssh_set_env(self) -> String {
        let mut environment = self.template.environment.clone();
        for variable in self.session_platform_env() {
            if let Some(value) = variable.value {
                environment.insert(variable.name, value);
            }
        }
        let assignments = environment
            .into_iter()
            .map(|(name, value)| sshd_argument(&name, &value))
            .collect::<Vec<_>>();
        if assignments.is_empty() {
            String::new()
        } else {
            format!("SetEnv {}\n", assignments.join(" "))
        }
    }

    pub fn resource_limits(&self) -> BTreeMap<String, Quantity> {
        quantities(
            format!("{}m", self.template.resources.cpu_millis),
            format!("{}Mi", self.template.resources.memory_mib),
            self.template
                .ephemeral_storage_limit_mib
                .map(|value| format!("{value}Mi")),
        )
    }

    pub fn workspace_init_container(&self, image: &str) -> Container {
        Container {
            name: "workspace-bootstrap".to_owned(),
            image: Some(image.to_owned()),
            command: Some(vec![BOOTSTRAP.to_owned()]),
            // The main container performs dependency detection and complete materialization. The
            // init container only creates PVC-backed layout and never assumes image packages.
            args: Some(vec!["prepare-layout".to_owned()]),
            env: Some(self.platform_env()),
            volume_mounts: Some(workspace_mounts(self.home, self.secondary_home())),
            security_context: Some(root_security_context(false)),
            ..Container::default()
        }
    }

    pub fn workspace_container(&self, image: &str, resources: ResourceRequirements) -> Container {
        let mut env = self.platform_env();
        env.extend(self.development_env());
        Container {
            name: "workspace".to_owned(),
            image: Some(image.to_owned()),
            command: Some(vec![BOOTSTRAP.to_owned()]),
            args: Some(vec!["serve".to_owned()]),
            ports: Some(vec![ContainerPort {
                container_port: 2222,
                name: Some("ssh".to_owned()),
                protocol: Some("TCP".to_owned()),
                ..ContainerPort::default()
            }]),
            readiness_probe: Some(Probe {
                tcp_socket: Some(TCPSocketAction {
                    port: IntOrString::Int(2222),
                    host: None,
                }),
                initial_delay_seconds: Some(1),
                period_seconds: Some(2),
                timeout_seconds: Some(1),
                success_threshold: Some(1),
                failure_threshold: Some(3),
                ..Probe::default()
            }),
            resources: Some(resources),
            env: Some(env),
            env_from: Some(injection_env_from()),
            volume_mounts: Some(self.development_mounts()),
            security_context: Some(root_security_context(false)),
            ..Container::default()
        }
    }

    pub fn buildkit_init_container(&self) -> Option<Container> {
        buildkit::init_container(self.has_buildkit())
    }

    pub fn buildkit_container(&self) -> Option<Container> {
        buildkit::container(self.has_buildkit())
    }

    pub fn affinity(&self) -> Option<Affinity> {
        if self.template.required_node_names.is_empty()
            && self.template.preferred_node_names.is_empty()
        {
            return None;
        }
        let required = (!self.template.required_node_names.is_empty()).then(|| NodeSelector {
            node_selector_terms: vec![hostname_term(&self.template.required_node_names)],
        });
        let preferred = (!self.template.preferred_node_names.is_empty()).then(|| {
            vec![PreferredSchedulingTerm {
                weight: 100,
                preference: hostname_term(&self.template.preferred_node_names),
            }]
        });
        Some(Affinity {
            node_affinity: Some(NodeAffinity {
                required_during_scheduling_ignored_during_execution: required,
                preferred_during_scheduling_ignored_during_execution: preferred,
            }),
            ..Affinity::default()
        })
    }

    pub fn node_selector(&self) -> Option<BTreeMap<String, String>> {
        (!self.template.node_selector.is_empty()).then(|| self.template.node_selector.clone())
    }

    pub fn pod_security_context(&self) -> Option<PodSecurityContext> {
        (self.template.preserve_home_ownership
            || self.template.buildkit
            || self.template.cluster_access)
            .then(|| PodSecurityContext {
                fs_group: Some(1000),
                fs_group_change_policy: Some("OnRootMismatch".to_owned()),
                seccomp_profile: Some(SeccompProfile {
                    type_: "RuntimeDefault".to_owned(),
                    ..SeccompProfile::default()
                }),
                ..PodSecurityContext::default()
            })
    }

    pub fn image_pull_secrets(&self) -> Option<Vec<LocalObjectReference>> {
        // The current Harbor library images are public. A future template field can name a
        // namespaced pull Secret without changing image identity semantics.
        None
    }

    fn has_buildkit(&self) -> bool {
        self.template.buildkit
    }

    fn secondary_home(&self) -> Option<&str> {
        None
    }

    fn platform_env(&self) -> Vec<EnvVar> {
        let mut environment = vec![
            env("MWC_WORKSPACE_USER", self.login_user),
            env("MWC_WORKSPACE_HOME", self.home),
            env(
                "MWC_IN_CLUSTER_KUBECONFIG",
                if self.template.cluster_access {
                    "true"
                } else {
                    "false"
                },
            ),
            env(
                "MWC_PRESERVE_HOME_ROOT",
                if self.template.preserve_home_ownership {
                    "true"
                } else {
                    "false"
                },
            ),
            env(
                "MWC_BUILDKIT_ENABLED",
                if self.template.buildkit {
                    "true"
                } else {
                    "false"
                },
            ),
        ];
        environment.extend(self.session_platform_env());
        environment
    }

    fn development_env(&self) -> Vec<EnvVar> {
        self.template
            .environment
            .iter()
            .filter(|(name, _)| !self.is_platform_environment(name))
            .map(|(name, value)| env(name, value))
            .collect()
    }

    fn session_platform_env(self) -> Vec<EnvVar> {
        let mut environment = vec![env("HOME", self.home)];
        if self.template.cluster_access {
            environment.push(env("KUBECONFIG", &format!("{}/.mwc/kubeconfig", self.home)));
        }
        if self.template.buildkit {
            environment.push(env(
                "BUILDKIT_HOST",
                &format!(
                    "unix://{}/.cache/buildkit/runtime/buildkit/buildkitd.sock",
                    self.home
                ),
            ));
        }
        environment
    }

    fn is_platform_environment(self, name: &str) -> bool {
        INTERNAL_PLATFORM_ENVIRONMENT.contains(&name)
            || name == "HOME"
            || self.template.cluster_access && name == "KUBECONFIG"
            || self.template.buildkit && name == "BUILDKIT_HOST"
    }

    fn development_mounts(&self) -> Vec<VolumeMount> {
        let mut mounts = workspace_mounts(self.home, self.secondary_home());
        if self.has_buildkit() {
            mounts.push(sub_path_mount("workspace-data", "/tmp", ".tmp/dev"));
            mounts.push(sub_path_mount("workspace-data", "/var/tmp", ".tmp/var"));
        }
        mounts
    }
}

pub(super) fn suppress_legacy_environment(
    template: &WorkspaceTemplateSpec,
    container: &mut Container,
    injected_targets: &std::collections::BTreeSet<String>,
) {
    let Some(environment) = container.env.as_mut() else {
        return;
    };
    let pod = WorkspacePod::from_template(template);
    environment.retain(|variable| {
        if pod.is_platform_environment(&variable.name) {
            return true;
        }
        !injected_targets.contains(&variable.name)
    });
}

fn hostname_term(values: &[String]) -> NodeSelectorTerm {
    NodeSelectorTerm {
        match_expressions: Some(vec![NodeSelectorRequirement {
            key: "kubernetes.io/hostname".to_owned(),
            operator: "In".to_owned(),
            values: Some(values.to_vec()),
        }]),
        ..NodeSelectorTerm::default()
    }
}

fn injection_env_from() -> Vec<EnvFromSource> {
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

fn quantities(
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

fn env(name: &str, value: &str) -> EnvVar {
    EnvVar {
        name: name.to_owned(),
        value: Some(value.to_owned()),
        ..EnvVar::default()
    }
}

fn sshd_argument(name: &str, value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{name}={escaped}\"")
}

fn sub_path_mount(name: &str, path: &str, sub_path: &str) -> VolumeMount {
    VolumeMount {
        name: name.to_owned(),
        mount_path: path.to_owned(),
        sub_path: Some(sub_path.to_owned()),
        read_only: Some(false),
        ..VolumeMount::default()
    }
}

fn root_security_context(read_only_root: bool) -> SecurityContext {
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

#[cfg(test)]
mod tests {
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
        assert!(config.contains("\"KUBECONFIG=/home/node-dev/.mwc/kubeconfig\""));
        assert!(config.contains(
            "\"BUILDKIT_HOST=unix:///home/node-dev/.cache/buildkit/runtime/buildkit/buildkitd.sock\""
        ));
        assert!(config.contains("\"TOOL_FLAGS=--name \\\"hello world\\\"\""));
    }
}
