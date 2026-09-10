use super::{ResourceBuilder, WorkspacePod};
use crate::kubernetes::resource_helpers::mount;
use k8s_openapi::{
    api::core::v1::{Container, ContainerPort, ResourceRequirements},
    apimachinery::pkg::api::resource::Quantity,
};
use std::collections::BTreeMap;

pub(super) fn container(
    builder: &ResourceBuilder,
    pod: WorkspacePod<'_>,
    route_key: &str,
) -> Container {
    let mut args = vec![
        "--port".to_owned(),
        "7681".to_owned(),
        "--writable".to_owned(),
        "--base-path".to_owned(),
        format!("/shell/{route_key}"),
    ];
    if builder.ttyd_mtls.is_some() {
        args.extend([
            "--ssl".to_owned(),
            "--ssl-cert".to_owned(),
            "/etc/mwc-ttyd-tls/tls.crt".to_owned(),
            "--ssl-key".to_owned(),
            "/etc/mwc-ttyd-tls/tls.key".to_owned(),
            "--ssl-ca".to_owned(),
            "/etc/mwc-ttyd-tls/ca.crt".to_owned(),
        ]);
    }
    args.extend([
        "/usr/bin/ssh".to_owned(),
        "-p".to_owned(),
        "2222".to_owned(),
        "-o".to_owned(),
        "StrictHostKeyChecking=yes".to_owned(),
        "-o".to_owned(),
        "UserKnownHostsFile=/etc/ssh/platform/known_hosts".to_owned(),
        "-o".to_owned(),
        "BatchMode=yes".to_owned(),
        "-i".to_owned(),
        "/etc/ssh/platform/ttyd_client_key".to_owned(),
        format!("{}@127.0.0.1", pod.login_user),
    ]);
    let mut volume_mounts = vec![
        mount("runtime-ssh", "/etc/ssh/platform", true),
        mount("ttyd-tmp", "/tmp", false),
        mount("ttyd-tmp", "/var/tmp", false),
    ];
    if builder.ttyd_mtls.is_some() {
        volume_mounts.push(mount("ttyd-tls", "/etc/mwc-ttyd-tls", true));
    }
    let mut container = Container {
        name: "ttyd".to_owned(),
        image: Some(builder.ttyd_image.clone()),
        command: Some(vec!["/usr/bin/ttyd".to_owned()]),
        args: Some(args),
        ports: Some(vec![ContainerPort {
            container_port: 7681,
            name: Some("web-shell".to_owned()),
            protocol: Some("TCP".to_owned()),
            ..ContainerPort::default()
        }]),
        volume_mounts: Some(volume_mounts),
        resources: Some(ResourceRequirements {
            requests: Some(BTreeMap::from([
                ("cpu".to_owned(), Quantity("10m".to_owned())),
                ("memory".to_owned(), Quantity("16Mi".to_owned())),
            ])),
            limits: Some(BTreeMap::from([
                ("cpu".to_owned(), Quantity("100m".to_owned())),
                ("memory".to_owned(), Quantity("128Mi".to_owned())),
            ])),
            ..ResourceRequirements::default()
        }),
        ..Container::default()
    };
    if builder.http_proxy_enabled() {
        container.command = Some(vec!["/usr/local/bin/mwc-ttyd-start".to_owned()]);
        container
            .volume_mounts
            .as_mut()
            .expect("ttyd has mounts")
            .push(mount("http-proxy-routes", "/etc/mwc-http-routes", true));
        container
            .ports
            .as_mut()
            .expect("ttyd has ports")
            .push(ContainerPort {
                container_port: crate::kubernetes::http_proxy::PORT,
                name: Some("app-proxy".to_owned()),
                ..ContainerPort::default()
            });
    }
    container
}
