use std::collections::BTreeMap;

use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::core::{ApiResource, DynamicObject, GroupVersionKind};

pub(super) fn http_route_resource() -> ApiResource {
    ApiResource::from_gvk(&GroupVersionKind::gvk(
        "gateway.networking.k8s.io",
        "v1",
        "HTTPRoute",
    ))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn web_shell_route(
    namespace: &str,
    labels: &BTreeMap<String, String>,
    workspace_short_id: &str,
    domain: &str,
    gateway_namespace: &str,
    gateway_name: &str,
    https_section_name: &str,
) -> DynamicObject {
    let path = format!("/shell/{workspace_short_id}/");
    let mut route = DynamicObject::new("web-shell", &http_route_resource())
        .within(namespace)
        .data(serde_json::json!({
            "spec": {
                "parentRefs": [{
                    "name": gateway_name,
                    "namespace": gateway_namespace,
                    "sectionName": https_section_name,
                }],
                "hostnames": [domain],
                "rules": [{
                    "matches": [{"path": {"type": "PathPrefix", "value": path}}],
                    "filters": [{
                        "type": "URLRewrite",
                        "urlRewrite": {"path": {
                            "type": "ReplacePrefixMatch",
                            "replacePrefixMatch": "/"
                        }}
                    }],
                    "backendRefs": [{"name": "workspace", "port": 7681}],
                }],
            }
        }));
    route.metadata = ObjectMeta {
        name: Some("web-shell".to_owned()),
        namespace: Some(namespace.to_owned()),
        labels: Some(labels.clone()),
        ..ObjectMeta::default()
    };
    route
}
