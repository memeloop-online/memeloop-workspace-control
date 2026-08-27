use std::collections::BTreeMap;

use k8s_openapi::api::networking::v1::{
    HTTPIngressPath, HTTPIngressRuleValue, Ingress, IngressBackend, IngressRule,
    IngressServiceBackend, IngressSpec, ServiceBackendPort,
};

use super::namespaced_metadata;

pub(super) fn web_shell_ingress(
    namespace: &str,
    labels: &BTreeMap<String, String>,
    workspace_short_id: &str,
    domain: &str,
) -> Ingress {
    let path = format!("/shell/{workspace_short_id}/");
    Ingress {
        metadata: namespaced_metadata("web-shell", namespace, labels),
        spec: Some(IngressSpec {
            ingress_class_name: Some("nginx".to_owned()),
            rules: Some(vec![IngressRule {
                host: Some(domain.to_owned()),
                http: Some(HTTPIngressRuleValue {
                    paths: vec![HTTPIngressPath {
                        backend: IngressBackend {
                            service: Some(IngressServiceBackend {
                                name: "workspace".to_owned(),
                                port: Some(ServiceBackendPort {
                                    number: Some(7681),
                                    ..ServiceBackendPort::default()
                                }),
                            }),
                            ..IngressBackend::default()
                        },
                        path: Some(path),
                        path_type: "Prefix".to_owned(),
                    }],
                }),
            }]),
            ..IngressSpec::default()
        }),
        ..Ingress::default()
    }
}
