use std::collections::BTreeMap;

use k8s_openapi::{
    ByteString,
    api::core::v1::{ConfigMap, Secret, Service, ServicePort, ServiceSpec, VolumeMount},
    apimachinery::pkg::{apis::meta::v1::ObjectMeta, util::intstr::IntOrString},
};

use super::COMPONENT_LABEL;

pub(super) fn service(
    namespace: &str,
    labels: &BTreeMap<String, String>,
    pod_labels: &BTreeMap<String, String>,
) -> Service {
    Service {
        metadata: namespaced_metadata("workspace", namespace, labels),
        spec: Some(ServiceSpec {
            type_: Some("ClusterIP".to_owned()),
            selector: Some(pod_labels.clone()),
            ports: Some(vec![
                service_port("ssh", 2222),
                service_port("web-shell", 7681),
            ]),
            ..ServiceSpec::default()
        }),
        ..Service::default()
    }
}

pub(super) fn pod_labels(labels: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut labels = labels.clone();
    labels.insert(COMPONENT_LABEL.to_owned(), "workspace".to_owned());
    labels
}

pub(crate) fn namespaced_metadata(
    name: &str,
    namespace: &str,
    labels: &BTreeMap<String, String>,
) -> ObjectMeta {
    ObjectMeta {
        name: Some(name.to_owned()),
        namespace: Some(namespace.to_owned()),
        labels: Some(labels.clone()),
        ..ObjectMeta::default()
    }
}

pub(super) fn mount(name: &str, path: &str, read_only: bool) -> VolumeMount {
    VolumeMount {
        name: name.to_owned(),
        mount_path: path.to_owned(),
        read_only: Some(read_only),
        ..VolumeMount::default()
    }
}

pub(super) fn workspace_mounts() -> Vec<VolumeMount> {
    vec![
        mount("workspace-data", "/workspace", false),
        mount("runtime-ssh", "/run/mwc-ssh", false),
        mount("ssh-identity", "/etc/ssh/platform", true),
        mount("workspace-config", "/etc/workspace-platform", true),
        mount(
            "workspace-files-secret",
            "/var/run/workspace-injections/secret",
            true,
        ),
        mount(
            "workspace-files-config",
            "/var/run/workspace-injections/config",
            true,
        ),
    ]
}

pub(super) fn workspace_config(namespace: &str, labels: &BTreeMap<String, String>) -> ConfigMap {
    ConfigMap {
        metadata: namespaced_metadata("workspace-config", namespace, labels),
        data: Some(BTreeMap::from([(
            "sshd_config".to_owned(),
            "Port 2222\nListenAddress 0.0.0.0\nHostKey /etc/ssh/platform/ssh_host_ed25519_key\nAuthorizedKeysFile /workspace/.mwc/authorized_keys\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nPermitRootLogin no\nAllowUsers workspace\nAllowTcpForwarding yes\nPermitTunnel no\nX11Forwarding no\nSubsystem sftp internal-sftp\nPidFile /run/mwc-ssh/sshd.pid\n".to_owned(),
        )])),
        ..ConfigMap::default()
    }
}

pub(super) fn ssh_identity(
    namespace: &str,
    labels: &BTreeMap<String, String>,
    identity: Option<&crate::storage::WorkspaceSshIdentity>,
) -> Secret {
    let data = identity.map(|identity| {
        BTreeMap::from([
            (
                "ssh_host_ed25519_key".to_owned(),
                ByteString(identity.private_key.as_bytes().to_vec()),
            ),
            (
                "ssh_host_ed25519_key.pub".to_owned(),
                ByteString(identity.public.public_key.as_bytes().to_vec()),
            ),
        ])
    });
    Secret {
        metadata: namespaced_metadata("workspace-ssh-identity", namespace, labels),
        data,
        type_: Some("Opaque".to_owned()),
        ..Secret::default()
    }
}

fn service_port(name: &str, port: i32) -> ServicePort {
    ServicePort {
        name: Some(name.to_owned()),
        port,
        protocol: Some("TCP".to_owned()),
        target_port: Some(IntOrString::Int(port)),
        ..ServicePort::default()
    }
}
