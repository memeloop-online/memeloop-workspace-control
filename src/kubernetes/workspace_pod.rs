use std::collections::BTreeMap;

use k8s_openapi::{
    api::core::v1::{
        Container, ContainerPort, EnvVar, ExecAction, LocalObjectReference, Probe,
        ResourceRequirements, VolumeMount,
    },
    apimachinery::pkg::api::resource::Quantity,
};

use crate::{templates::WorkspaceTemplateSpec, workspace_runtime::WorkspaceResourceNames};

use super::{
    buildkit,
    resource_helpers::{mount, workspace_mounts},
};

mod support;

use support::{env, injection_env_from, quantities, root_security_context, sshd_argument};

const BOOTSTRAP: &str = "/etc/workspace-platform/mwc-workspace-bootstrap";
const SCRATCH_ROOT: &str = "/var/lib/mwc/workspace-scratch";
const WORKSPACE_CACHE_SUB_PATH: &str = "workspace-cache";
const CODEX_SESSION_SUB_PATH: &str = "codex-session-scratch";
const BUILD_SCRATCH: &str = "/var/lib/mwc/build-scratch";
const CODEX_SCRATCH: &str = "/var/lib/mwc/codex-scratch";
const BUILDKIT_VOLUME_MOUNT: &str = "/run/mwc-buildkit";
const INTERNAL_PLATFORM_ENVIRONMENT: [&str; 8] = [
    "MWC_WORKSPACE_USER",
    "MWC_WORKSPACE_HOME",
    "MWC_IN_CLUSTER_KUBECONFIG",
    "MWC_BUILDKIT_ENABLED",
    "MWC_BUILD_SCRATCH",
    "MWC_HOME_RESERVE_MIB",
    "MWC_DESKTOP_ENABLED",
    "MWC_DESKTOP_PORT",
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
            Some("512Mi".to_owned()),
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
        quantities(
            format!("{}m", self.template.resources.cpu_millis),
            format!("{}Mi", self.template.resources.memory_mib),
            Some("2048Mi".to_owned()),
        )
    }

    pub fn workspace_scratch_init_container(&self, image: &str) -> Container {
        Container {
            name: "workspace-scratch-init".to_owned(),
            image: Some(image.to_owned()),
            command: Some(vec!["sh".to_owned(), "-c".to_owned()]),
            args: Some(vec![scratch_init_script().to_owned()]),
            env: Some(vec![
                env("MWC_WORKSPACE_USER", self.login_user),
                env(
                    "MWC_BUILDKIT_ENABLED",
                    if self.template.buildkit {
                        "true"
                    } else {
                        "false"
                    },
                ),
            ]),
            volume_mounts: Some(vec![mount(buildkit::SCRATCH_VOLUME, SCRATCH_ROOT, false)]),
            resources: Some(ResourceRequirements {
                requests: Some(quantities("10m", "16Mi", None)),
                limits: Some(quantities("100m", "64Mi", None)),
                ..ResourceRequirements::default()
            }),
            security_context: Some(root_security_context(true)),
            ..Container::default()
        }
    }

    pub fn workspace_init_container(
        &self,
        image: &str,
        resources: ResourceRequirements,
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
            resources: Some(resources),
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
        let mut ports = vec![ContainerPort {
            container_port: 2222,
            name: Some("ssh".to_owned()),
            protocol: Some("TCP".to_owned()),
            ..ContainerPort::default()
        }];
        if let Some(desktop) = &self.template.desktop {
            ports.push(ContainerPort {
                container_port: i32::from(desktop.internal_port),
                name: Some("desktop".to_owned()),
                protocol: Some("TCP".to_owned()),
                ..ContainerPort::default()
            });
        }
        Container {
            name: "workspace".to_owned(),
            image: Some(image.to_owned()),
            command: Some(vec![BOOTSTRAP.to_owned()]),
            args: Some(vec!["serve".to_owned()]),
            ports: Some(ports),
            readiness_probe: Some(Probe {
                exec: Some(ExecAction {
                    command: Some(vec![
                        "sh".to_owned(),
                        "-c".to_owned(),
                        self.readiness_check(),
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
        buildkit::container(self.has_buildkit())
    }

    pub fn buildkit_bootstrap_container(&self) -> Option<Container> {
        buildkit::bootstrap_container(self.has_buildkit())
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

    fn readiness_check(&self) -> String {
        let sshd = "test -s /run/mwc-ssh/sshd.pid && kill -0 \"$(cat /run/mwc-ssh/sshd.pid)\"";
        if self.template.desktop.is_some() {
            format!(
                "{sshd} && test -s /run/mwc-ssh/desktop.pid && kill -0 \"$(cat /run/mwc-ssh/desktop.pid)\""
            )
        } else {
            sshd.to_owned()
        }
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
                &fixed_home_disk_margin_mib(self.template.resources.disk_gib).to_string(),
            ),
        ];
        environment.extend(self.session_platform_env());
        if let Some(desktop) = &self.template.desktop {
            environment.extend([
                env("MWC_DESKTOP_ENABLED", "true"),
                env("MWC_DESKTOP_PORT", &desktop.internal_port.to_string()),
            ]);
        }
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
            environment.push(env("BUILDKIT_HOST", buildkit::ENDPOINT));
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
            scratch_sub_path_mount(BUILD_SCRATCH, WORKSPACE_CACHE_SUB_PATH),
            scratch_sub_path_mount(CODEX_SCRATCH, CODEX_SESSION_SUB_PATH),
        ]);
        if self.has_buildkit() {
            mounts.push(scratch_sub_path_mount(
                BUILDKIT_VOLUME_MOUNT,
                buildkit::CACHE_SUB_PATH,
            ));
        }
        mounts
    }
}

fn scratch_sub_path_mount(path: &str, sub_path: &str) -> VolumeMount {
    VolumeMount {
        name: buildkit::SCRATCH_VOLUME.to_owned(),
        mount_path: path.to_owned(),
        sub_path: Some(sub_path.to_owned()),
        read_only: Some(false),
        ..VolumeMount::default()
    }
}

fn fixed_home_disk_margin_mib(disk_gib: u64) -> u64 {
    disk_gib.saturating_mul(1_024).saturating_div(10).min(1_024)
}

fn scratch_init_script() -> &'static str {
    r#"set -eu
scratch_root=/var/lib/mwc/workspace-scratch
workspace_group=$(id -gn "$MWC_WORKSPACE_USER")

prepare_directory() {
    path=$1
    owner=$2
    group=$3
    if [ -L "$path" ] || { [ -e "$path" ] && [ ! -d "$path" ]; }; then
        echo "unsafe workspace scratch path: $path" >&2
        exit 70
    fi
    mkdir -p -- "$path"
    chown "$owner:$group" -- "$path"
    chmod 0700 -- "$path"
}

if [ -L "$scratch_root" ] || [ ! -d "$scratch_root" ]; then
    echo "workspace scratch volume is not mounted" >&2
    exit 70
fi
prepare_directory "$scratch_root/workspace-cache" "$MWC_WORKSPACE_USER" "$workspace_group"
prepare_directory "$scratch_root/codex-session-scratch" "$MWC_WORKSPACE_USER" "$workspace_group"
if [ "$MWC_BUILDKIT_ENABLED" = true ]; then
    prepare_directory "$scratch_root/build-cache" 1000 1000
fi
"#
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
