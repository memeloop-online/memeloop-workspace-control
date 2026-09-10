use std::collections::BTreeMap;

use k8s_openapi::{
    api::networking::v1::{
        IPBlock, NetworkPolicy, NetworkPolicyEgressRule, NetworkPolicyIngressRule,
        NetworkPolicyPeer, NetworkPolicyPort, NetworkPolicySpec,
    },
    apimachinery::pkg::{apis::meta::v1::LabelSelector, util::intstr::IntOrString},
};

use super::{InternetEgressConfig, namespaced_metadata};
use crate::{
    templates::EgressPolicy, workspace_runtime::WorkspaceRuntimeNames, workspaces::AccessMode,
};

const PRIVATE_OR_RESERVED_IPV4: &[&str] = &[
    "0.0.0.0/8",
    "10.0.0.0/8",
    "100.64.0.0/10",
    "127.0.0.0/8",
    "169.254.0.0/16",
    "172.16.0.0/12",
    "192.0.0.0/24",
    "192.0.2.0/24",
    "192.31.196.0/24",
    "192.52.193.0/24",
    "192.88.99.0/24",
    "192.175.48.0/24",
    "192.168.0.0/16",
    "198.18.0.0/15",
    "198.51.100.0/24",
    "203.0.113.0/24",
    "224.0.0.0/4",
    "240.0.0.0/4",
];

const PRIVATE_OR_RESERVED_IPV6: &[&str] = &["2001::/23", "2001:db8::/32", "2002::/16", "3fff::/20"];

#[allow(clippy::too_many_arguments)]
pub(super) fn build(
    runtime: &WorkspaceRuntimeNames,
    ownership_labels: &BTreeMap<String, String>,
    pod_labels: &BTreeMap<String, String>,
    higress_namespace: &str,
    higress_pod_labels: &BTreeMap<String, String>,
    higress_source_cidrs: &[String],
    internet_egress: Option<&InternetEgressConfig>,
    jump_host_namespace: &str,
    jump_host_pod_labels: &BTreeMap<String, String>,
    access_mode: AccessMode,
    egress_policy: EgressPolicy,
    internal_ssh_node_port_enabled: bool,
) -> NetworkPolicy {
    let ssh_rule = match access_mode {
        AccessMode::Public => ingress_rule(jump_host_namespace, jump_host_pod_labels, 2222),
        AccessMode::Internal => internal_cluster_ssh_rule(internal_ssh_node_port_enabled),
    };
    let internet_only = egress_policy == EgressPolicy::InternetOnly;
    NetworkPolicy {
        metadata: namespaced_metadata(
            &runtime.resources.network_policy,
            &runtime.namespace,
            ownership_labels,
        ),
        spec: Some(NetworkPolicySpec {
            pod_selector: Some(LabelSelector {
                match_labels: Some(pod_labels.clone()),
                ..LabelSelector::default()
            }),
            policy_types: Some(
                ["Ingress", "Egress"]
                    .into_iter()
                    .take(if internet_only { 2 } else { 1 })
                    .map(str::to_owned)
                    .collect(),
            ),
            ingress: Some(vec![
                ingress_rule_with_ip_blocks(
                    higress_namespace,
                    higress_pod_labels,
                    higress_source_cidrs,
                    7681,
                ),
                ssh_rule,
            ]),
            egress: internet_only.then(|| {
                internet_egress_rules(internet_egress.expect(
                    "ResourceBuilder rejects internet_only templates without egress config",
                ))
            }),
        }),
    }
}

fn internet_egress_rules(config: &InternetEgressConfig) -> Vec<NetworkPolicyEgressRule> {
    let blocked_cidrs = config.blocked_cidrs();
    let mut rules = vec![dns_egress_rule(
        &config.dns_namespace,
        &config.dns_pod_labels,
    )];
    if let Some(rule) = public_egress_rule("0.0.0.0/0", PRIVATE_OR_RESERVED_IPV4, &blocked_cidrs) {
        rules.push(rule);
    }
    if let Some(rule) = public_egress_rule("2000::/3", PRIVATE_OR_RESERVED_IPV6, &blocked_cidrs) {
        rules.push(rule);
    }
    rules
}

fn dns_egress_rule(
    namespace: &str,
    pod_labels: &BTreeMap<String, String>,
) -> NetworkPolicyEgressRule {
    NetworkPolicyEgressRule {
        to: Some(vec![NetworkPolicyPeer {
            namespace_selector: Some(LabelSelector {
                match_labels: Some(BTreeMap::from([(
                    "kubernetes.io/metadata.name".to_owned(),
                    namespace.to_owned(),
                )])),
                ..LabelSelector::default()
            }),
            pod_selector: Some(LabelSelector {
                match_labels: Some(pod_labels.clone()),
                ..LabelSelector::default()
            }),
            ..NetworkPolicyPeer::default()
        }]),
        ports: Some(vec![network_port("UDP", 53), network_port("TCP", 53)]),
    }
}

fn public_egress_rule(
    cidr: &str,
    default_except: &[&str],
    blocked_cidrs: &[ipnet::IpNet],
) -> Option<NetworkPolicyEgressRule> {
    let allowed = cidr
        .parse::<ipnet::IpNet>()
        .expect("static public CIDR is valid");
    let ipv4 = allowed.addr().is_ipv4();
    if blocked_cidrs
        .iter()
        .any(|blocked| blocked.contains(&allowed))
    {
        return None;
    }
    let mut except = default_except
        .iter()
        .map(|cidr| (*cidr).to_owned())
        .collect::<Vec<_>>();
    except.extend(
        blocked_cidrs
            .iter()
            .filter(|blocked| blocked.addr().is_ipv4() == ipv4 && allowed.contains(*blocked))
            .map(ToString::to_string),
    );
    Some(NetworkPolicyEgressRule {
        to: Some(vec![NetworkPolicyPeer {
            ip_block: Some(IPBlock {
                cidr: cidr.to_owned(),
                except: Some(except),
            }),
            ..NetworkPolicyPeer::default()
        }]),
        ..NetworkPolicyEgressRule::default()
    })
}

fn network_port(protocol: &str, port: i32) -> NetworkPolicyPort {
    NetworkPolicyPort {
        port: Some(IntOrString::Int(port)),
        protocol: Some(protocol.to_owned()),
        ..NetworkPolicyPort::default()
    }
}

pub(super) fn ingress_rule_with_ip_blocks(
    namespace: &str,
    pod_labels: &BTreeMap<String, String>,
    source_cidrs: &[String],
    port: i32,
) -> NetworkPolicyIngressRule {
    let mut rule = ingress_rule(namespace, pod_labels, port);
    rule.from
        .get_or_insert_default()
        .extend(source_cidrs.iter().map(|cidr| NetworkPolicyPeer {
            ip_block: Some(IPBlock {
                cidr: cidr.clone(),
                except: None,
            }),
            ..NetworkPolicyPeer::default()
        }));
    rule
}

fn internal_cluster_ssh_rule(tailnet_enabled: bool) -> NetworkPolicyIngressRule {
    let mut peers = vec![NetworkPolicyPeer {
        // An empty namespace selector matches all cluster namespaces while
        // still excluding traffic that did not enter through Kubernetes.
        namespace_selector: Some(LabelSelector::default()),
        ..NetworkPolicyPeer::default()
    }];
    if tailnet_enabled {
        peers.push(NetworkPolicyPeer {
            ip_block: Some(IPBlock {
                cidr: "100.64.0.0/10".to_owned(),
                except: None,
            }),
            ..NetworkPolicyPeer::default()
        });
    }
    NetworkPolicyIngressRule {
        from: Some(peers),
        ports: Some(vec![NetworkPolicyPort {
            port: Some(IntOrString::Int(2222)),
            protocol: Some("TCP".to_owned()),
            ..NetworkPolicyPort::default()
        }]),
    }
}

fn ingress_rule(
    namespace: &str,
    pod_labels: &BTreeMap<String, String>,
    port: i32,
) -> NetworkPolicyIngressRule {
    NetworkPolicyIngressRule {
        from: Some(vec![NetworkPolicyPeer {
            namespace_selector: Some(LabelSelector {
                match_labels: Some(BTreeMap::from([(
                    "kubernetes.io/metadata.name".to_owned(),
                    namespace.to_owned(),
                )])),
                ..LabelSelector::default()
            }),
            pod_selector: Some(LabelSelector {
                match_labels: Some(pod_labels.clone()),
                ..LabelSelector::default()
            }),
            ..NetworkPolicyPeer::default()
        }]),
        ports: Some(vec![NetworkPolicyPort {
            port: Some(IntOrString::Int(port)),
            protocol: Some("TCP".to_owned()),
            ..NetworkPolicyPort::default()
        }]),
    }
}
