use std::collections::BTreeMap;

use k8s_openapi::{
    api::core::v1::{
        AppArmorProfile, Capabilities, Container, EnvVar, ExecAction, Probe, ResourceRequirements,
        SeccompProfile, SecurityContext, VolumeMount,
    },
    apimachinery::pkg::api::resource::Quantity,
};

pub(super) const ENDPOINT: &str = "tcp://127.0.0.1:1234";
pub(super) const SCRATCH_VOLUME: &str = "workspace-scratch";
pub(super) const CACHE_SUB_PATH: &str = "build-cache";

pub(super) fn bootstrap_container(enabled: bool, image: &str) -> Option<Container> {
    enabled.then(|| Container {
        name: "buildkit-bootstrap".to_owned(),
        image: Some(image.to_owned()),
        command: Some(vec!["sh".to_owned(), "-c".to_owned()]),
        args: Some(vec![setup_script().to_owned()]),
        resources: Some(ResourceRequirements {
            requests: Some(quantities("10m", "16Mi", "32Mi")),
            limits: Some(quantities("100m", "128Mi", "128Mi")),
            ..ResourceRequirements::default()
        }),
        volume_mounts: Some(vec![cache_mount("/var/lib/mwc-buildkit", CACHE_SUB_PATH)]),
        security_context: Some(non_root_security_context(false, false)),
        ..Container::default()
    })
}

fn buildkit_tmp_mount() -> VolumeMount {
    cache_mount("/tmp", "build-cache/tmp")
}

pub(super) fn container(enabled: bool, image: &str) -> Option<Container> {
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
        image: Some(image.to_owned()),
        // Keep the rootless image entrypoint. It starts buildkitd through RootlessKit and creates
        // the user namespace required by the OCI worker.
        args: Some(vec![
            "--config".to_owned(),
            "/var/lib/mwc-buildkit/config/buildkitd.toml".to_owned(),
        ]),
        env: Some(vec![
            env("TMPDIR", "/var/lib/mwc-buildkit/tmp"),
            env("XDG_RUNTIME_DIR", "/var/lib/mwc-buildkit/runtime"),
            env("BUILDKIT_HOST", ENDPOINT),
        ]),
        readiness_probe: Some(probe.clone()),
        liveness_probe: Some(Probe {
            initial_delay_seconds: Some(10),
            ..probe
        }),
        resources: Some(ResourceRequirements {
            requests: Some(quantities("250m", "512Mi", "256Mi")),
            limits: Some(quantities("4", "4Gi", "1Gi")),
            ..ResourceRequirements::default()
        }),
        volume_mounts: Some(vec![
            cache_mount("/var/lib/mwc-buildkit", CACHE_SUB_PATH),
            buildkit_tmp_mount(),
        ]),
        security_context: Some(non_root_security_context(true, true)),
        ..Container::default()
    })
}

fn cache_mount(path: &str, sub_path: &str) -> VolumeMount {
    VolumeMount {
        name: SCRATCH_VOLUME.to_owned(),
        mount_path: path.to_owned(),
        sub_path: Some(sub_path.to_owned()),
        read_only: Some(false),
        ..VolumeMount::default()
    }
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

fn setup_script() -> &'static str {
    r#"set -eu
mkdir -p /var/lib/mwc-buildkit/bin /var/lib/mwc-buildkit/config \
  /var/lib/mwc-buildkit/runtime /var/lib/mwc-buildkit/state /var/lib/mwc-buildkit/tmp
chmod 0700 /var/lib/mwc-buildkit/runtime
cp /usr/bin/buildctl /var/lib/mwc-buildkit/bin/buildctl
chmod 0555 /var/lib/mwc-buildkit/bin/buildctl
cat > /var/lib/mwc-buildkit/config/buildkitd.toml <<'CONFIG'
root = "/var/lib/mwc-buildkit/state"
[grpc]
  address = ["tcp://127.0.0.1:1234"]
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
