use std::collections::BTreeMap;

use k8s_openapi::api::networking::v1::{
    HTTPIngressPath, HTTPIngressRuleValue, Ingress, IngressBackend, IngressRule,
    IngressServiceBackend, IngressSpec, ServiceBackendPort,
};

use super::namespaced_metadata;
use crate::workspace_runtime::WorkspaceRuntimeIdentity;

pub(super) fn web_shell_ingress(
    runtime: &WorkspaceRuntimeIdentity,
    labels: &BTreeMap<String, String>,
    domain: &str,
) -> Ingress {
    let names = runtime.names();
    Ingress {
        metadata: namespaced_metadata(&names.web_shell_ingress, &runtime.namespace, labels),
        spec: Some(IngressSpec {
            ingress_class_name: Some("nginx".to_owned()),
            rules: Some(vec![IngressRule {
                host: Some(domain.to_owned()),
                http: Some(HTTPIngressRuleValue {
                    paths: vec![HTTPIngressPath {
                        backend: IngressBackend {
                            service: Some(IngressServiceBackend {
                                name: names.service,
                                port: Some(ServiceBackendPort {
                                    number: Some(7681),
                                    ..ServiceBackendPort::default()
                                }),
                            }),
                            ..IngressBackend::default()
                        },
                        path: Some(runtime.web_shell_path()),
                        path_type: "Prefix".to_owned(),
                    }],
                }),
            }]),
            ..IngressSpec::default()
        }),
        ..Ingress::default()
    }
}
