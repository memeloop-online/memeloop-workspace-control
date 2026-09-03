use std::collections::BTreeMap;

use k8s_openapi::{
    api::networking::v1::{
        IPBlock, NetworkPolicy, NetworkPolicyIngressRule, NetworkPolicyPeer, NetworkPolicyPort,
        NetworkPolicySpec,
    },
    apimachinery::pkg::{apis::meta::v1::LabelSelector, util::intstr::IntOrString},
};

use super::namespaced_metadata;
use crate::workspaces::AccessMode;

#[allow(clippy::too_many_arguments)]
pub(super) fn build(
    namespace: &str,
    ownership_labels: &BTreeMap<String, String>,
    pod_labels: &BTreeMap<String, String>,
    higress_namespace: &str,
    higress_pod_labels: &BTreeMap<String, String>,
    higress_source_cidrs: &[String],
    jump_host_namespace: &str,
    jump_host_pod_labels: &BTreeMap<String, String>,
    access_mode: AccessMode,
    internal_ssh_node_port_enabled: bool,
) -> NetworkPolicy {
    let ssh_rule = match access_mode {
        AccessMode::Public => ingress_rule(jump_host_namespace, jump_host_pod_labels, 2222),
        AccessMode::Internal => internal_cluster_ssh_rule(internal_ssh_node_port_enabled),
    };
    NetworkPolicy {
        metadata: namespaced_metadata("workspace-ingress", namespace, ownership_labels),
        spec: Some(NetworkPolicySpec {
            pod_selector: Some(LabelSelector {
                match_labels: Some(pod_labels.clone()),
                ..LabelSelector::default()
            }),
            policy_types: Some(vec!["Ingress".to_owned()]),
            ingress: Some(vec![
                ingress_rule_with_ip_blocks(
                    higress_namespace,
                    higress_pod_labels,
                    higress_source_cidrs,
                    7681,
                ),
                ssh_rule,
            ]),
            ..NetworkPolicySpec::default()
        }),
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
