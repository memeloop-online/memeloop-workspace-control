use std::collections::BTreeMap;

use k8s_openapi::{
    api::core::v1::{
        AppArmorProfile, Capabilities, Container, EnvVar, ExecAction, Probe, ResourceRequirements,
        SeccompProfile, SecurityContext, VolumeMount,
    },
    apimachinery::pkg::api::resource::Quantity,
};

use super::resource_helpers::mount;

pub(super) const IMAGE: &str = "harbor.k3s.onetwo.website/docker-io/moby/buildkit:v0.32.2-rootless@sha256:504731e577c20559c00f968f33219f30115e70be29ab96728d1d06e963fc494b";

pub(super) fn init_container(enabled: bool) -> Option<Container> {
    enabled.then(|| Container {
        name: "buildctl".to_owned(),
        image: Some(IMAGE.to_owned()),
        command: Some(vec!["sh".to_owned(), "-c".to_owned()]),
        args: Some(vec![init_script().to_owned()]),
        volume_mounts: Some(vec![mount("workspace-data", "/mnt/home", false)]),
        security_context: Some(non_root_security_context(false, false)),
        ..Container::default()
    })
}

pub(super) fn container(enabled: bool) -> Option<Container> {
    if !enabled {
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
        image: Some(IMAGE.to_owned()),
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
            requests: Some(quantities("250m", "512Mi", "128Mi")),
            limits: Some(quantities("4", "4Gi", "512Mi")),
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

fn quantities(cpu: &str, memory: &str, ephemeral: &str) -> BTreeMap<String, Quantity> {
    BTreeMap::from([
        ("cpu".to_owned(), Quantity(cpu.to_owned())),
        ("memory".to_owned(), Quantity(memory.to_owned())),
        (
            "ephemeral-storage".to_owned(),
            Quantity(ephemeral.to_owned()),
        ),
    ])
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

fn init_script() -> &'static str {
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
