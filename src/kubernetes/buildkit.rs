use std::collections::BTreeMap;

use k8s_openapi::{
    api::core::v1::{
        AppArmorProfile, Container, EnvVar, ExecAction, Probe, ResourceRequirements,
        SeccompProfile, SecurityContext, VolumeMount,
    },
    apimachinery::pkg::api::resource::Quantity,
};

pub(super) const ENDPOINT: &str = "tcp://127.0.0.1:1234";
pub(super) const SCRATCH_VOLUME: &str = "workspace-scratch";
pub(super) const CACHE_SUB_PATH: &str = "build-cache";

fn buildkit_tmp_mount() -> VolumeMount {
    cache_mount("/tmp", "build-cache/tmp")
}

pub(super) fn container(enabled: bool, image: &str, cpu_request_millis: u64) -> Option<Container> {
    if !enabled {
        return None;
    }
    let workers = ExecAction {
        command: Some(vec![
            "buildctl".to_owned(),
            "debug".to_owned(),
            "workers".to_owned(),
        ]),
    };
    Some(Container {
        name: "buildkitd".to_owned(),
        image: Some(image.to_owned()),
        // Prepare the build cache and client alongside SSH, then enter the same
        // RootlessKit user namespace as the pinned image's default entrypoint.
        command: Some(vec!["sh".to_owned(), "-c".to_owned()]),
        args: Some(vec![start_script().to_owned()]),
        env: Some(vec![
            env("TMPDIR", "/var/lib/mwc-buildkit/tmp"),
            env("XDG_RUNTIME_DIR", "/var/lib/mwc-buildkit/runtime"),
            env("BUILDKIT_HOST", ENDPOINT),
        ]),
        // Wait for a usable worker at startup. Once it has started, a slow build should not
        // make the whole workspace lose readiness while SSH and the terminal still work.
        startup_probe: Some(Probe {
            exec: Some(workers.clone()),
            period_seconds: Some(2),
            timeout_seconds: Some(3),
            failure_threshold: Some(30),
            ..Probe::default()
        }),
        liveness_probe: Some(Probe {
            exec: Some(workers),
            period_seconds: Some(30),
            timeout_seconds: Some(5),
            ..Probe::default()
        }),
        resources: Some(ResourceRequirements {
            requests: Some(quantities(
                &format!("{cpu_request_millis}m"),
                "512Mi",
                "256Mi",
            )),
            limits: Some(quantities("4", "4Gi", "1Gi")),
            ..ResourceRequirements::default()
        }),
        volume_mounts: Some(vec![
            cache_mount("/var/lib/mwc-buildkit", CACHE_SUB_PATH),
            // Override the rootless image's VOLUME so CRI cannot allocate a node-local volume.
            cache_mount("/home/user/.local/share/buildkit", CACHE_SUB_PATH),
            buildkit_tmp_mount(),
        ]),
        security_context: Some(security_context()),
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

fn security_context() -> SecurityContext {
    SecurityContext {
        run_as_user: Some(1000),
        run_as_group: Some(1000),
        run_as_non_root: Some(true),
        privileged: Some(false),
        allow_privilege_escalation: Some(true),
        read_only_root_filesystem: Some(true),
        app_armor_profile: Some(AppArmorProfile {
            type_: "Unconfined".to_owned(),
        }),
        seccomp_profile: Some(SeccompProfile {
            type_: "Unconfined".to_owned(),
            ..SeccompProfile::default()
        }),
        ..SecurityContext::default()
    }
}

fn start_script() -> &'static str {
    r#"set -eu
mkdir -p /var/lib/mwc-buildkit/bin /var/lib/mwc-buildkit/config \
  /var/lib/mwc-buildkit/runtime /var/lib/mwc-buildkit/state /var/lib/mwc-buildkit/tmp
chmod 0700 /var/lib/mwc-buildkit/runtime
# Publish the complete client in one rename. A BuildKit restart may occur
# while a workspace session is already using the previous executable.
client=$(mktemp /var/lib/mwc-buildkit/bin/.buildctl.XXXXXX)
config=
trap 'rm -f -- "$client" "$config"' EXIT
cp /usr/bin/buildctl "$client"
chmod 0555 "$client"
mv -fT -- "$client" /var/lib/mwc-buildkit/bin/buildctl
config=$(mktemp /var/lib/mwc-buildkit/config/.buildkitd.XXXXXX)
cat > "$config" <<'CONFIG'
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
mv -fT -- "$config" /var/lib/mwc-buildkit/config/buildkitd.toml
exec /usr/bin/rootlesskit /usr/bin/buildkitd --config /var/lib/mwc-buildkit/config/buildkitd.toml
"#
}
