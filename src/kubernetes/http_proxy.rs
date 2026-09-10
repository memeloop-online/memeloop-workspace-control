use k8s_openapi::{
    api::{
        core::v1::{ConfigMap, Service},
        networking::v1::{Ingress, NetworkPolicy},
    },
    apimachinery::pkg::util::intstr::IntOrString,
};
use kube::core::DynamicObject;
use serde_json::json;
use std::collections::BTreeMap;

use super::{
    BuildError, ResourceBuilder, TtydMtlsConfig, envoy_filter, higress, namespaced_metadata,
    port_mappings,
};
use crate::{
    storage::PortMapping, workspace_runtime::WorkspaceRuntimeNames, workspaces::Workspace,
};

pub(super) const PORT: i32 = 8443;

pub(super) fn secure_mapping(
    service: &mut Service,
    ingress: &mut Ingress,
    policy: &mut NetworkPolicy,
    runtime: &WorkspaceRuntimeNames,
    mtls: &TtydMtlsConfig,
) {
    service
        .spec
        .as_mut()
        .expect("generated mapping service has a spec")
        .ports
        .as_mut()
        .expect("generated mapping service has a port")[0]
        .target_port = Some(IntOrString::Int(PORT));
    ingress.metadata.annotations = Some(higress::upstream_tls_annotations(runtime, mtls));
    for rule in policy
        .spec
        .as_mut()
        .expect("generated policy has a spec")
        .ingress
        .as_mut()
        .expect("generated policy has ingress rules")
    {
        for port in rule.ports.as_mut().expect("generated ingress has a port") {
            port.port = Some(IntOrString::Int(PORT));
        }
    }
}

impl ResourceBuilder {
    pub(super) fn http_proxy_enabled(&self) -> bool {
        self.ttyd_mtls.is_some() && self.port_mapping_domain.is_some()
    }

    pub(super) fn http_proxy_routes(
        &self,
        workspace: &Workspace,
        mappings: &[PortMapping],
    ) -> Result<Option<ConfigMap>, BuildError> {
        if !self.http_proxy_enabled() {
            return Ok(None);
        }
        let runtime = self.runtime_names(workspace)?;
        let domain = self
            .port_mapping_domain
            .as_deref()
            .expect("proxy domain is configured");
        let mut routes = BTreeMap::new();
        for mapping in mappings {
            if mapping.workspace_id == workspace.id
                && mapping.organization_id == workspace.organization_id
            {
                routes.insert(
                    port_mappings::hostname(mapping, domain),
                    mapping.internal_port,
                );
            }
        }
        let content = routes
            .into_iter()
            .map(|(host, port)| {
                let host = serde_json::to_string(&host).expect("hostname string serializes");
                format!("{host} {port};\n")
            })
            .collect::<String>();
        Ok(Some(ConfigMap {
            metadata: namespaced_metadata(
                &runtime.resources.http_proxy_config,
                &runtime.namespace,
                &self.workspace_labels(workspace),
            ),
            data: Some(BTreeMap::from([("ports.conf".to_owned(), content)])),
            ..ConfigMap::default()
        }))
    }

    pub(super) fn port_mapping_tls_filter(
        &self,
        workspace: &Workspace,
        mapping: &PortMapping,
    ) -> Result<Option<DynamicObject>, BuildError> {
        let Some(mtls) = self
            .ttyd_mtls
            .as_ref()
            .filter(|_| self.http_proxy_enabled())
        else {
            return Ok(None);
        };
        let runtime = self.runtime_names(workspace)?;
        let mut labels = self.workspace_labels(workspace);
        labels.insert(
            port_mappings::PORT_MAPPING_ID_LABEL.to_owned(),
            mapping.id.to_string(),
        );
        let mut filter = envoy_filter::web_shell_san_filter(
            &runtime,
            &labels,
            &self.higress_namespace,
            &self.higress_pod_labels,
            mtls,
        );
        filter.metadata.name = Some(format!("{}-san", port_mappings::name(mapping)));
        // Pin the certificate to the workspace identity, even though this
        // cluster targets a mapping Service rather than the SSH/ttyd Service.
        filter.data["spec"]["configPatches"][0]["match"]["cluster"] = json!({
            "service": format!("{}.{}.svc.cluster.local", port_mappings::name(mapping), runtime.namespace),
            "portNumber": mapping.internal_port,
        });
        Ok(Some(filter))
    }
}
