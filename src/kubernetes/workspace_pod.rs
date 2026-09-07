use std::collections::BTreeMap;

use k8s_openapi::{
    api::core::v1::{
        Affinity, Container, ContainerPort, EnvVar, ExecAction, LocalObjectReference, NodeAffinity,
        NodeSelector, PreferredSchedulingTerm, Probe, ResourceRequirements, VolumeMount,
    },
    apimachinery::pkg::api::resource::Quantity,
};

use crate::{templates::WorkspaceTemplateSpec, workspace_runtime::WorkspaceResourceNames};

use super::{
    buildkit,
    resource_helpers::{mount, workspace_mounts},
};

mod support;

use support::{
    env, hostname_term, injection_env_from, quantities, root_security_context, sshd_argument,
};

const BOOTSTRAP: &str = "/etc/workspace-platform/mwc-workspace-bootstrap";
const BUILD_SCRATCH: &str = "/var/lib/mwc/build-scratch";
const CODEX_SCRATCH: &str = "/var/lib/mwc/codex-scratch";
const BUILDKIT_VOLUME_MOUNT: &str = "/run/mwc-buildkit";
const BUILDKIT_RUNTIME: &str = "/run/mwc-buildkit/runtime";
const INTERNAL_PLATFORM_ENVIRONMENT: [&str; 6] = [
    "MWC_WORKSPACE_USER",
    "MWC_WORKSPACE_HOME",
    "MWC_IN_CLUSTER_KUBECONFIG",
    "MWC_BUILDKIT_ENABLED",
    "MWC_BUILD_SCRATCH",
    "MWC_HOME_RESERVE_MIB",
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
        "yes"
    }

    pub fn ssh_set_env(self) -> String {
        let assignments = self
            .session_platform_env()
            .into_iter()
            .filter_map(|variable| {
                variable
                    .value
                    .map(|value| sshd_argument(&variable.name, &value))
            })
            .collect::<Vec<_>>();
        if assignments.is_empty() {
            String::new()
        } else {
            format!("SetEnv {}\n", assignments.join(" "))
        }
    }

    pub fn resource_limits(&self) -> BTreeMap<String, Quantity> {
        let policy_limit_mib = self
            .template
            .storage_policy
            .build_scratch_gib
            .saturating_add(self.template.storage_policy.codex_scratch_gib)
            .saturating_mul(1_024)
            .saturating_add(256);
        let ephemeral_limit = self
            .template
            .ephemeral_storage_limit_mib
            .unwrap_or_default()
            .max(policy_limit_mib);
        quantities(
            format!("{}m", self.template.resources.cpu_millis),
            format!("{}Mi", self.template.resources.memory_mib),
            Some(format!("{ephemeral_limit}Mi")),
        )
    }

    pub fn workspace_init_container(
        &self,
        image: &str,
        names: &WorkspaceResourceNames,
    ) -> Container {
        Container {
            name: "workspace-bootstrap".to_owned(),
            image: Some(image.to_owned()),
            command: Some(vec![BOOTSTRAP.to_owned()]),
            // The main container performs dependency detection and complete materialization. The
            // init container only creates PVC-backed layout and never assumes image packages.
            args: Some(vec!["prepare-layout".to_owned()]),
            env: Some(self.platform_env()),
            volume_mounts: Some(self.development_mounts(names)),
            security_context: Some(root_security_context(false)),
            ..Container::default()
        }
    }

    pub fn workspace_container(
        &self,
        image: &str,
        resources: ResourceRequirements,
        names: &WorkspaceResourceNames,
    ) -> Container {
        let env = self.platform_env();
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
                exec: Some(ExecAction {
                    command: Some(vec![
                        "sh".to_owned(),
                        "-c".to_owned(),
                        "test -s /run/mwc-ssh/sshd.pid && kill -0 \"$(cat /run/mwc-ssh/sshd.pid)\""
                            .to_owned(),
                    ]),
                }),
                initial_delay_seconds: Some(1),
                period_seconds: Some(10),
                timeout_seconds: Some(1),
                success_threshold: Some(1),
                failure_threshold: Some(3),
                ..Probe::default()
            }),
            resources: Some(resources),
            env: Some(env),
            env_from: Some(injection_env_from(names)),
            volume_mounts: Some(self.development_mounts(names)),
            security_context: Some(root_security_context(false)),
            ..Container::default()
        }
    }

    pub fn buildkit_container(&self) -> Option<Container> {
        buildkit::container(
            self.has_buildkit(),
            self.template.storage_policy.buildkit_cache_gib,
        )
    }

    pub fn buildkit_bootstrap_container(&self) -> Option<Container> {
        buildkit::bootstrap_container(self.has_buildkit())
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

    pub fn pod_security_context(&self) -> Option<k8s_openapi::api::core::v1::PodSecurityContext> {
        None
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
                "MWC_BUILDKIT_ENABLED",
                if self.template.buildkit {
                    "true"
                } else {
                    "false"
                },
            ),
            env("MWC_BUILD_SCRATCH", BUILD_SCRATCH),
            env(
                "MWC_HOME_RESERVE_MIB",
                &self
                    .template
                    .storage_policy
                    .effective_home_reserve_mib(self.template.resources.disk_gib)
                    .to_string(),
            ),
        ];
        environment.extend(self.session_platform_env());
        environment
    }

    fn session_platform_env(self) -> Vec<EnvVar> {
        let mut environment = vec![
            env(
                "PATH",
                &format!(
                    "{}/.local/bin:{}/.local/share/pnpm:{}/.cargo/bin:/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
                    self.home, self.home, self.home,
                ),
            ),
            env("HOME", self.home),
            env("RUSTUP_HOME", "/usr/local/rustup"),
            env("TMPDIR", &format!("{BUILD_SCRATCH}/tmp")),
            env("TMP", &format!("{BUILD_SCRATCH}/tmp")),
            env("TEMP", &format!("{BUILD_SCRATCH}/tmp")),
            env("XDG_CACHE_HOME", &format!("{BUILD_SCRATCH}/cache")),
            env("CARGO_TARGET_DIR", &format!("{BUILD_SCRATCH}/cargo-target")),
        ];
        if self.template.cluster_access {
            environment.push(env("KUBECONFIG", "/run/mwc-ssh/kubeconfig"));
        }
        if self.template.buildkit {
            environment[0] = env(
                "PATH",
                &format!(
                    "/run/mwc-buildkit/bin:{}/.local/bin:{}/.local/share/pnpm:{}/.cargo/bin:/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
                    self.home, self.home, self.home,
                ),
            );
            environment.push(env(
                "BUILDKIT_HOST",
                &format!("unix://{BUILDKIT_RUNTIME}/buildkit/buildkitd.sock"),
            ));
        }
        environment
    }

    fn is_platform_environment(self, name: &str) -> bool {
        INTERNAL_PLATFORM_ENVIRONMENT.contains(&name)
            || matches!(name, "PATH" | "HOME" | "RUSTUP_HOME" | "CARGO_HOME")
            || matches!(
                name,
                "TMPDIR" | "TMP" | "TEMP" | "XDG_CACHE_HOME" | "CARGO_TARGET_DIR"
            )
            || self.template.cluster_access && name == "KUBECONFIG"
            || self.template.buildkit && name == "BUILDKIT_HOST"
    }

    fn development_mounts(&self, names: &WorkspaceResourceNames) -> Vec<VolumeMount> {
        let mut mounts =
            workspace_mounts(&names.data_claim_template, self.home, self.secondary_home());
        mounts.extend([
            mount("runtime-tmp", "/tmp", false),
            mount("runtime-tmp", "/var/tmp", false),
            mount("build-scratch", BUILD_SCRATCH, false),
            mount("codex-scratch", CODEX_SCRATCH, false),
        ]);
        if self.has_buildkit() {
            mounts.push(mount("buildkit-cache", BUILDKIT_VOLUME_MOUNT, false));
        }
        mounts
    }
}

pub(super) fn apply_injected_environment_overrides(
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

#[cfg(test)]
mod tests;
