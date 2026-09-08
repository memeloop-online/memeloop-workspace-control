use std::collections::BTreeMap;

use kube::core::{ApiResource, DynamicObject, GroupVersionKind};
use serde_json::json;

use super::{TtydMtlsConfig, namespaced_metadata};
use crate::workspace_runtime::WorkspaceRuntimeNames;

pub(super) fn api_resource() -> ApiResource {
    ApiResource::from_gvk(&GroupVersionKind::gvk(
        "networking.istio.io",
        "v1alpha3",
        "EnvoyFilter",
    ))
}

pub(super) fn web_shell_san_filter(
    runtime: &WorkspaceRuntimeNames,
    labels: &BTreeMap<String, String>,
    gateway_namespace: &str,
    gateway_labels: &BTreeMap<String, String>,
    mtls: &TtydMtlsConfig,
) -> DynamicObject {
    let resource = api_resource();
    let service_fqdn = format!(
        "{}.{}.svc.cluster.local",
        runtime.resources.service, runtime.namespace
    );
    let companion = format!("{}-cacert", mtls.higress_client_secret_name);
    let mut filter = DynamicObject::new(&runtime.resources.web_shell_envoy_filter, &resource).data(json!({
        "spec": {
            "workloadSelector": { "labels": gateway_labels },
            "configPatches": [{
                "applyTo": "CLUSTER",
                "match": {
                    "context": "GATEWAY",
                    "cluster": { "service": service_fqdn, "portNumber": 7681 }
                },
                "patch": {
                    "operation": "MERGE",
                    "value": {
                        "transport_socket": {
                            "name": "envoy.transport_sockets.tls",
                            "typed_config": {
                                "@type": "type.googleapis.com/envoy.extensions.transport_sockets.tls.v3.UpstreamTlsContext",
                                "common_tls_context": {
                                    "combined_validation_context": {
                                        "default_validation_context": {
                                            "match_typed_subject_alt_names": [{
                                                "san_type": "DNS",
                                                "matcher": { "exact": service_fqdn }
                                            }]
                                        },
                                        "validation_context_sds_secret_config": {
                                            "name": format!("kubernetes-ingress://Kubernetes/{gateway_namespace}/{companion}"),
                                            "sds_config": { "ads": {}, "resource_api_version": "V3" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }]
        }
    }));
    filter.metadata = namespaced_metadata(
        &runtime.resources.web_shell_envoy_filter,
        gateway_namespace,
        labels,
    );
    filter
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::InstallationId, workspace_runtime::WorkspaceRuntimeIdentity};

    #[test]
    fn filter_is_exact_and_preserves_higress_sds_shape() {
        let runtime = WorkspaceRuntimeNames::for_workspace(
            &"public-a".parse::<InstallationId>().unwrap(),
            &WorkspaceRuntimeIdentity::new(uuid::Uuid::nil(), "0000000000000000").unwrap(),
            "0000000000000000",
        )
        .unwrap();
        let filter = web_shell_san_filter(
            &runtime,
            &BTreeMap::new(),
            "higress-system",
            &BTreeMap::from([("app".into(), "higress-gateway".into())]),
            &TtydMtlsConfig::new("server".into(), "higress-system".into(), "client".into())
                .unwrap(),
        );
        let value = serde_json::to_value(filter).unwrap();
        let patch = &value["spec"]["configPatches"][0];
        assert_eq!(patch["match"]["context"], "GATEWAY");
        assert_eq!(patch["match"]["cluster"]["portNumber"], 7681);
        assert_eq!(
            patch["match"]["cluster"]["service"],
            "w-0000000000000000.memeloop-workspace-control.svc.cluster.local"
        );
        let combined = &patch["patch"]["value"]["transport_socket"]["typed_config"]["common_tls_context"]
            ["combined_validation_context"];
        assert_eq!(
            combined["default_validation_context"]["match_typed_subject_alt_names"][0]["matcher"]["exact"],
            "w-0000000000000000.memeloop-workspace-control.svc.cluster.local"
        );
        assert_eq!(
            combined["validation_context_sds_secret_config"]["name"],
            "kubernetes-ingress://Kubernetes/higress-system/client-cacert"
        );
        assert!(
            patch
                .to_string()
                .contains("tls_certificate_sds_secret_configs")
                == false
        );
    }
}
