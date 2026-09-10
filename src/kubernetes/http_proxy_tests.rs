use super::fixture;
use crate::{
    kubernetes::{ResourceBuilder, TtydMtlsConfig, port_mappings},
    storage::PortMapping,
    workspaces::Workspace,
};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use uuid::Uuid;

fn configured() -> (ResourceBuilder, Workspace, PortMapping) {
    let (mut builder, workspace) = fixture();
    builder.port_mapping_domain = Some("apps.example".to_owned());
    builder
        .higress_pod_labels
        .insert("app".to_owned(), "higress-gateway".to_owned());
    builder.ttyd_mtls = Some(
        TtydMtlsConfig::new(
            "server".to_owned(),
            "ca".to_owned(),
            "higress-system".to_owned(),
            "client".to_owned(),
        )
        .unwrap(),
    );
    let mapping = PortMapping {
        id: Uuid::now_v7(),
        organization_id: workspace.organization_id,
        workspace_id: workspace.id,
        internal_port: 3000,
        display_name: None,
        created_by: workspace.owner_id,
        created_at: 1,
    };
    (builder, workspace, mapping)
}

#[test]
fn authenticated_mapping_opens_only_the_proxy_port() {
    let (builder, workspace, mapping) = configured();
    let (service, ingress, policy) = builder
        .port_mapping_resources(&workspace, &mapping)
        .unwrap()
        .unwrap();
    let ports = service.spec.unwrap().ports.unwrap();
    assert_eq!(ports[0].port, 3000);
    assert_eq!(ports[0].target_port, Some(IntOrString::Int(8443)));
    let annotations = ingress.metadata.annotations.unwrap();
    assert_eq!(
        annotations["nginx.ingress.kubernetes.io/backend-protocol"],
        "HTTPS"
    );
    assert_eq!(
        annotations["nginx.ingress.kubernetes.io/proxy-ssl-verify"],
        "on"
    );
    let allowed = policy
        .spec
        .unwrap()
        .ingress
        .unwrap()
        .into_iter()
        .flat_map(|rule| rule.ports.unwrap())
        .map(|port| port.port)
        .collect::<Vec<_>>();
    assert_eq!(allowed, vec![Some(IntOrString::Int(8443))]);
}

#[test]
fn route_allowlist_is_workspace_scoped_and_empty_after_removal() {
    let (builder, workspace, mapping) = configured();
    let mut foreign = mapping.clone();
    foreign.id = Uuid::now_v7();
    foreign.workspace_id = Uuid::now_v7();
    let routes = builder
        .http_proxy_routes(&workspace, &[mapping.clone(), foreign])
        .unwrap()
        .unwrap();
    assert_eq!(
        routes.data.unwrap()["ports.conf"],
        format!(
            "\"{}\" 3000;\n",
            port_mappings::hostname(&mapping, "apps.example")
        )
    );
    let empty = builder.http_proxy_routes(&workspace, &[]).unwrap().unwrap();
    assert_eq!(empty.data.unwrap()["ports.conf"], "");
}

#[test]
fn proxy_mounts_stay_in_terminal_container_and_use_optional_projected_routes() {
    let (builder, workspace, _) = configured();
    let pod = builder
        .build(&workspace)
        .unwrap()
        .stateful_set
        .spec
        .unwrap()
        .template
        .spec
        .unwrap();
    let ttyd = pod.containers.iter().find(|c| c.name == "ttyd").unwrap();
    assert_eq!(
        ttyd.command.as_ref().unwrap(),
        &["/usr/local/bin/mwc-ttyd-start"]
    );
    let main = pod
        .containers
        .iter()
        .find(|c| c.name == "workspace")
        .unwrap();
    assert!(
        main.volume_mounts
            .as_ref()
            .unwrap()
            .iter()
            .all(|mount| mount.name != "http-proxy-routes" && mount.name != "ttyd-tls")
    );
    let volume = pod
        .volumes
        .unwrap()
        .into_iter()
        .find(|v| v.name == "http-proxy-routes")
        .unwrap();
    assert_eq!(volume.config_map.unwrap().optional, Some(true));
}

#[test]
fn mapping_cluster_pins_the_workspace_certificate_identity() {
    let (builder, workspace, mapping) = configured();
    let filter = builder
        .port_mapping_tls_filter(&workspace, &mapping)
        .unwrap()
        .unwrap();
    let patch = &filter.data["spec"]["configPatches"][0];
    assert_eq!(patch["match"]["cluster"]["portNumber"], 3000);
    assert_eq!(
        patch["match"]["cluster"]["service"],
        format!(
            "{}.memeloop-workspace-control.svc.cluster.local",
            port_mappings::name(&mapping)
        )
    );
    assert_eq!(
        patch["patch"]["value"]["transport_socket"]["typed_config"]["common_tls_context"]["combined_validation_context"]
            ["default_validation_context"]["match_typed_subject_alt_names"][0]["matcher"]["exact"],
        "w-8000000000000001.memeloop-workspace-control.svc.cluster.local"
    );
    assert_eq!(
        filter.metadata.labels.unwrap()[port_mappings::PORT_MAPPING_ID_LABEL],
        mapping.id.to_string()
    );
}
