use std::collections::BTreeMap;

use k8s_openapi::{
    api::core::v1::{
        Affinity, AppArmorProfile, Capabilities, ConfigMapEnvSource, Container, ContainerPort,
        EnvFromSource, EnvVar, ExecAction, LocalObjectReference, NodeAffinity, NodeSelector,
        NodeSelectorRequirement, NodeSelectorTerm, PodSecurityContext, PreferredSchedulingTerm,
        Probe, ResourceRequirements, SeccompProfile, SecretEnvSource, SecurityContext,
        TCPSocketAction, VolumeMount,
    },
    apimachinery::pkg::{api::resource::Quantity, util::intstr::IntOrString},
};

use crate::templates::WorkspaceTemplateSpec;

use super::resource_helpers::{mount, workspace_mounts};

pub(super) const BUILDKIT_IMAGE: &str = "harbor.k3s.onetwo.website/docker-io/moby/buildkit:v0.32.2-rootless@sha256:504731e577c20559c00f968f33219f30115e70be29ab96728d1d06e963fc494b";
const BOOTSTRAP: &str = "/etc/workspace-platform/mwc-workspace-bootstrap";
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
        if self.template.preserve_home_root {
            // Migrated Coder PVC roots are intentionally root:1000/2775. The platform owns the
            // generated authorized_keys file and keeps it 0600, so retaining the legacy root
            // metadata requires disabling only sshd's parent-directory ownership check.
            "no"
        } else {
            "yes"
        }
    }

    pub fn ssh_set_env(self) -> String {
        let assignments = self
            .development_env()
            .into_iter()
            .filter_map(|variable| {
                variable
                    .value
                    .map(|value| format!("{}={value}", variable.name))
            })
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
        if !self.has_buildkit() {
            return None;
        }
        Some(Container {
            name: "buildctl".to_owned(),
            image: Some(BUILDKIT_IMAGE.to_owned()),
            command: Some(vec!["sh".to_owned(), "-c".to_owned()]),
            args: Some(vec![buildkit_init_script().to_owned()]),
            volume_mounts: Some(vec![mount("workspace-data", "/mnt/home", false)]),
            security_context: Some(non_root_security_context(false, false)),
            ..Container::default()
        })
    }

    pub fn buildkit_container(&self) -> Option<Container> {
        if !self.has_buildkit() {
            return None;
        }
        let probe = Probe {
            exec: Some(ExecAction {
                command: Some(vec![
                    "buildctl".to_owned(),
                    "debug".to_owned(),
                    "workers".to_owned(),
                ]),
            }),
            initial_delay_seconds: Some(5),
            period_seconds: Some(30),
            ..Probe::default()
        };
        Some(Container {
            name: "buildkitd".to_owned(),
            image: Some(BUILDKIT_IMAGE.to_owned()),
            args: Some(vec![
                "--config".to_owned(),
                "/home/user/.config/buildkit/buildkitd.toml".to_owned(),
            ]),
            env: Some(vec![
                env("TMPDIR", "/tmp"),
                env("XDG_RUNTIME_DIR", "/run/user/1000"),
            ]),
            readiness_probe: Some(probe.clone()),
            liveness_probe: Some(Probe {
                initial_delay_seconds: Some(10),
                ..probe
            }),
            resources: Some(ResourceRequirements {
                requests: Some(quantities("250m", "512Mi", Some("128Mi".to_owned()))),
                limits: Some(quantities("4", "4Gi", Some("512Mi".to_owned()))),
                ..ResourceRequirements::default()
            }),
            volume_mounts: Some(vec![
                sub_path_mount(
                    "workspace-data",
                    "/home/user/.config/buildkit",
                    ".config/buildkit",
                ),
                sub_path_mount(
                    "workspace-data",
                    "/home/user/.local/share/buildkit",
                    ".cache/buildkit/state",
                ),
                sub_path_mount(
                    "workspace-data",
                    "/run/user/1000",
                    ".cache/buildkit/runtime",
                ),
                sub_path_mount("workspace-data", "/tmp", ".tmp/buildkit"),
                sub_path_mount("workspace-data", "/var/tmp", ".tmp/buildkit"),
            ]),
            security_context: Some(non_root_security_context(true, true)),
            ..Container::default()
        })
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
        (self.template.preserve_home_root || self.template.buildkit || self.template.cluster_access)
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
        vec![
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
                if self.template.preserve_home_root {
                    "true"
                } else {
                    "false"
                },
            ),
        ]
    }

    fn development_env(&self) -> Vec<EnvVar> {
        self.template
            .environment
            .iter()
            .map(|(name, value)| env(name, value))
            .collect()
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

fn non_root_security_context(
    allow_privilege_escalation: bool,
    unconfined: bool,
) -> SecurityContext {
    SecurityContext {
        run_as_user: Some(1000),
        run_as_group: Some(1000),
        run_as_non_root: Some(true),
        privileged: Some(false),
        allow_privilege_escalation: Some(allow_privilege_escalation),
        read_only_root_filesystem: Some(true),
        capabilities: (!allow_privilege_escalation).then(|| Capabilities {
            drop: Some(vec!["ALL".to_owned()]),
            ..Capabilities::default()
        }),
        app_armor_profile: unconfined.then(|| AppArmorProfile {
            type_: "Unconfined".to_owned(),
            ..AppArmorProfile::default()
        }),
        seccomp_profile: Some(SeccompProfile {
            type_: if unconfined {
                "Unconfined".to_owned()
            } else {
                "RuntimeDefault".to_owned()
            },
            ..SeccompProfile::default()
        }),
        ..SecurityContext::default()
    }
}

fn buildkit_init_script() -> &'static str {
    r#"set -eu
mkdir -p /mnt/home/.local/bin /mnt/home/.cache/buildkit/state \
  /mnt/home/.cache/buildkit/runtime/buildkit /mnt/home/.tmp/dev \
  /mnt/home/.tmp/buildkit /mnt/home/.tmp/var /mnt/home/workspace \
  /mnt/home/.cargo/registry /mnt/home/.cargo/git \
  /mnt/home/.cache/cargo-target /mnt/home/.config/buildkit \
  /mnt/home/.config/docker /mnt/home/.cache/ms-playwright \
  /mnt/home/.cache/npm /mnt/home/.cache/yarn /mnt/home/.local/share/pnpm \
  /mnt/home/.local/share /mnt/home/.local/state
cp /usr/bin/buildctl /mnt/home/.local/bin/buildctl
chmod 0555 /mnt/home/.local/bin/buildctl
cat > /mnt/home/.config/buildkit/buildkitd.toml <<'CONFIG'
root = "/home/user/.local/share/buildkit"
[grpc]
  address = ["unix:///run/user/1000/buildkit/buildkitd.sock"]
[cdi]
  disabled = true
[worker.oci]
  enabled = true
  rootless = true
  noProcessSandbox = true
  snapshotter = "native"
  gc = true
  reservedSpace = "10%"
  maxUsedSpace = "45%"
  minFreeSpace = "20%"
  max-parallelism = 4
[worker.containerd]
  enabled = false
CONFIG
"#
}
