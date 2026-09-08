use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use std::collections::BTreeMap;

use k8s_openapi::api::networking::v1::{
    HTTPIngressPath, HTTPIngressRuleValue, Ingress, IngressBackend, IngressRule,
    IngressServiceBackend, IngressSpec, ServiceBackendPort,
};

use super::{TtydMtlsConfig, namespaced_metadata};
use crate::workspace_runtime::WorkspaceRuntimeNames;

pub(super) fn web_shell_ingress(
    runtime: &WorkspaceRuntimeNames,
    labels: &BTreeMap<String, String>,
    domain: &str,
    ttyd_mtls: Option<&TtydMtlsConfig>,
) -> Ingress {
    let annotations = ttyd_mtls.map(|mtls| {
        BTreeMap::from([
            (
                "nginx.ingress.kubernetes.io/backend-protocol".to_owned(),
                "HTTPS".to_owned(),
            ),
            (
                "nginx.ingress.kubernetes.io/proxy-ssl-secret".to_owned(),
                format!(
                    "{}/{}",
                    mtls.higress_client_secret_namespace, mtls.higress_client_secret_name
                ),
            ),
            (
                "nginx.ingress.kubernetes.io/proxy-ssl-name".to_owned(),
                format!(
                    "{}.{}.svc.cluster.local",
                    runtime.resources.service, runtime.namespace
                ),
            ),
            (
                "nginx.ingress.kubernetes.io/proxy-ssl-server-name".to_owned(),
                "on".to_owned(),
            ),
            (
                "nginx.ingress.kubernetes.io/proxy-ssl-verify".to_owned(),
                "on".to_owned(),
            ),
        ])
    });
    Ingress {
        metadata: ObjectMeta {
            annotations,
            ..namespaced_metadata(
                &runtime.resources.web_shell_ingress,
                &runtime.namespace,
                labels,
            )
        },
        spec: Some(IngressSpec {
            ingress_class_name: Some("nginx".to_owned()),
            rules: Some(vec![IngressRule {
                host: Some(domain.to_owned()),
                http: Some(HTTPIngressRuleValue {
                    paths: vec![HTTPIngressPath {
                        backend: IngressBackend {
                            service: Some(IngressServiceBackend {
                                name: runtime.resources.service.clone(),
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
