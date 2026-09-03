//! Kubernetes objects for authenticated workspace HTTP port mappings.

use std::collections::BTreeMap;

use k8s_openapi::{
    api::{
        core::v1::{Service, ServicePort, ServiceSpec},
        networking::v1::{
            HTTPIngressPath, HTTPIngressRuleValue, Ingress, IngressBackend, IngressRule,
            IngressServiceBackend, IngressSpec, IngressTLS, NetworkPolicy, ServiceBackendPort,
        },
    },
    apimachinery::pkg::{apis::meta::v1::LabelSelector, util::intstr::IntOrString},
};

use crate::storage::PortMapping;

use super::{namespaced_metadata, network_policy::ingress_rule_with_ip_blocks};

pub const PORT_MAPPING_ID_LABEL: &str = "workspace.memeloop.dev/port-mapping-id";

/// Names are derived from UUIDs, never user input.  The Service is ClusterIP
/// only; its port and targetPort both point at the workspace pod port.
pub fn resources(
    namespace: &str,
    labels: &BTreeMap<String, String>,
    pod_labels: &BTreeMap<String, String>,
    wildcard_domain: &str,
    mapping: &PortMapping,
) -> (Service, Ingress) {
    let name = name(mapping);
    let hostname = hostname(mapping, wildcard_domain);
    let labels = mapping_labels(labels, mapping);
    let service = Service {
        metadata: namespaced_metadata(&name, namespace, &labels),
        spec: Some(ServiceSpec {
            type_: Some("ClusterIP".to_owned()),
            selector: Some(pod_labels.clone()),
            ports: Some(vec![ServicePort {
                name: Some("http".to_owned()),
                port: i32::from(mapping.internal_port),
                target_port: Some(IntOrString::Int(i32::from(mapping.internal_port))),
                protocol: Some("TCP".to_owned()),
                ..ServicePort::default()
            }]),
            ..ServiceSpec::default()
        }),
        ..Service::default()
    };
    let ingress = Ingress {
        metadata: namespaced_metadata(&name, namespace, &labels),
        spec: Some(IngressSpec {
            ingress_class_name: Some("nginx".to_owned()),
            // The placeholder is deliberately absent from user namespaces.
            // Higress fallbackForInvalidSecret resolves the host against its
            // central credentialConfig, so wildcard private keys never enter a
            // workspace namespace.
            tls: Some(vec![IngressTLS {
                hosts: Some(vec![hostname.clone()]),
                secret_name: Some("mwc-port-mapping-tls".to_owned()),
            }]),
            // A unique hostname deliberately avoids a path rewrite.  SPA asset
            // URLs, service-worker scope, WebSockets and absolute redirects all
            // retain their normal application-visible paths.
            rules: Some(vec![IngressRule {
                host: Some(hostname),
                http: Some(HTTPIngressRuleValue {
                    paths: vec![HTTPIngressPath {
                        path: Some("/".to_owned()),
                        path_type: "Prefix".to_owned(),
                        backend: IngressBackend {
                            service: Some(IngressServiceBackend {
                                name: name.clone(),
                                port: Some(ServiceBackendPort {
                                    number: Some(i32::from(mapping.internal_port)),
                                    ..ServiceBackendPort::default()
                                }),
                            }),
                            ..IngressBackend::default()
                        },
                    }],
                }),
            }]),
            ..IngressSpec::default()
        }),
        ..Ingress::default()
    };
    (service, ingress)
}

pub fn name(mapping: &PortMapping) -> String {
    format!("port-{}", mapping.id.simple())
}

/// Requires a wildcard DNS record and wildcard TLS certificate for
/// `*.{wildcard_domain}`.  UUID-derived labels are DNS-safe and never reveal
/// a user supplied workspace name or application port.
pub fn hostname(mapping: &PortMapping, wildcard_domain: &str) -> String {
    format!("p-{}.{}", mapping.id.simple(), wildcard_domain)
}

/// This additional policy is intentionally separate from the base workspace
/// policy: Kubernetes combines policies additively, so it opens only this
/// declared application port to Higress.  It never opens it to a node, host or
/// arbitrary namespace.
pub fn network_policy(
    namespace: &str,
    labels: &BTreeMap<String, String>,
    pod_labels: &BTreeMap<String, String>,
    higress_namespace: &str,
    higress_pod_labels: &BTreeMap<String, String>,
    higress_source_cidrs: &[String],
    mapping: &PortMapping,
) -> NetworkPolicy {
    let labels = mapping_labels(labels, mapping);
    NetworkPolicy {
        metadata: namespaced_metadata(&format!("{}-ingress", name(mapping)), namespace, &labels),
        spec: Some(k8s_openapi::api::networking::v1::NetworkPolicySpec {
            pod_selector: Some(LabelSelector {
                match_labels: Some(pod_labels.clone()),
                ..LabelSelector::default()
            }),
            policy_types: Some(vec!["Ingress".to_owned()]),
            ingress: Some(vec![ingress_rule_with_ip_blocks(
                higress_namespace,
                higress_pod_labels,
                higress_source_cidrs,
                i32::from(mapping.internal_port),
            )]),
            ..k8s_openapi::api::networking::v1::NetworkPolicySpec::default()
        }),
    }
}

fn mapping_labels(
    labels: &BTreeMap<String, String>,
    mapping: &PortMapping,
) -> BTreeMap<String, String> {
    let mut owned = labels.clone();
    owned.insert(PORT_MAPPING_ID_LABEL.to_owned(), mapping.id.to_string());
    owned
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    #[test]
    fn never_creates_a_node_port() {
        let mapping = PortMapping {
            id: Uuid::nil(),
            organization_id: Uuid::nil(),
            workspace_id: Uuid::nil(),
            internal_port: 3000,
            display_name: None,
            created_by: Uuid::nil(),
            created_at: 1,
        };
        let (service, ingress) = resources(
            "ns",
            &BTreeMap::new(),
            &BTreeMap::new(),
            "ports.example.test",
            &mapping,
        );
        assert_eq!(service.spec.unwrap().type_.as_deref(), Some("ClusterIP"));
        let ingress_spec = ingress.spec.unwrap();
        assert_eq!(
            ingress_spec.rules.unwrap()[0].host.as_deref(),
            Some("p-00000000000000000000000000000000.ports.example.test")
        );
        let tls = &ingress_spec.tls.unwrap()[0];
        assert_eq!(
            tls.hosts.as_deref(),
            Some(["p-00000000000000000000000000000000.ports.example.test".to_owned()].as_slice())
        );
        assert_eq!(tls.secret_name.as_deref(), Some("mwc-port-mapping-tls"));
    }

    #[test]
    fn allows_host_network_higress_source_cidrs() {
        let mapping = PortMapping {
            id: Uuid::nil(),
            organization_id: Uuid::nil(),
            workspace_id: Uuid::nil(),
            internal_port: 3000,
            display_name: None,
            created_by: Uuid::nil(),
            created_at: 1,
        };
        let gateway_labels = BTreeMap::from([("app".to_owned(), "higress-gateway".to_owned())]);
        let policy = network_policy(
            "workspace",
            &BTreeMap::new(),
            &BTreeMap::new(),
            "higress-system",
            &gateway_labels,
            &["10.50.0.0/24".to_owned(), "10.42.0.0/16".to_owned()],
            &mapping,
        );
        let mut rules = policy.spec.unwrap().ingress.unwrap();
        let rule = rules.remove(0);
        assert_eq!(rule.ports.unwrap()[0].port, Some(IntOrString::Int(3000)));
        let peers = rule.from.unwrap();
        assert!(peers.iter().any(|peer| {
            peer.namespace_selector.is_some()
                && peer
                    .pod_selector
                    .as_ref()
                    .and_then(|selector| selector.match_labels.as_ref())
                    == Some(&gateway_labels)
        }));
        assert_eq!(
            peers
                .iter()
                .filter_map(|peer| peer.ip_block.as_ref().map(|block| block.cidr.as_str()))
                .collect::<Vec<_>>(),
            ["10.50.0.0/24", "10.42.0.0/16"]
        );

        let policy_without_cidrs = network_policy(
            "workspace",
            &BTreeMap::new(),
            &BTreeMap::new(),
            "higress-system",
            &gateway_labels,
            &[],
            &mapping,
        );
        let peers = policy_without_cidrs.spec.unwrap().ingress.unwrap()[0]
            .from
            .clone()
            .unwrap();
        assert_eq!(peers.len(), 1);
        assert!(peers[0].ip_block.is_none());
    }
}
