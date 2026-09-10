use std::collections::BTreeSet;

use k8s_openapi::api::core::v1::{ConfigMap, Pod};
use kube::{
    Api,
    api::{DeleteParams, ListParams, Patch, PatchParams, Preconditions},
    core::DynamicObject,
};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::super::super::{
    OWNER_INSTALLATION_LABEL, WORKSPACE_ID_LABEL, envoy_filter, port_mappings,
};
use super::super::{FIELD_MANAGER, KubernetesCoordinator, ReconcileError, verify_existing};
use crate::{storage::PortMapping, workspaces::Workspace};

impl KubernetesCoordinator {
    pub(super) async fn apply_http_proxy_routes(
        &self,
        workspace: &Workspace,
        mappings: &[PortMapping],
    ) -> Result<(), ReconcileError> {
        let Some(config) = self.builder.http_proxy_routes(workspace, mappings)? else {
            return Ok(());
        };
        let names = self.builder.runtime_names(workspace)?;
        let api = Api::<ConfigMap>::namespaced(self.client.clone(), workspace.runtime.namespace());
        let name = &names.resources.http_proxy_config;
        verify_existing(&api, name, &self.builder, workspace.id).await?;
        api.patch(
            name,
            &PatchParams::apply(FIELD_MANAGER),
            &Patch::Apply(&config),
        )
        .await?;
        let content = config
            .data
            .as_ref()
            .and_then(|data| data.get("ports.conf"))
            .map(String::as_str)
            .unwrap_or_default();
        let revision = format!("{:x}", Sha256::digest(content.as_bytes()));
        let pods = Api::<Pod>::namespaced(self.client.clone(), workspace.runtime.namespace());
        let pod_name = names.resources.pod_ordinal_zero();
        if let Some(pod) = pods.get_metadata_opt(&pod_name).await? {
            self.builder
                .verify_delete_ownership(&pod.metadata, workspace.id)?;
            let key = "workspace.memeloop.dev/http-routes";
            if pod.metadata.deletion_timestamp.is_none()
                && pod
                    .metadata
                    .annotations
                    .as_ref()
                    .and_then(|values| values.get(key))
                    != Some(&revision)
            {
                // A metadata update prompts kubelet to refresh projected volumes;
                // it neither changes the StatefulSet template nor replaces the Pod.
                let patch = json!({"metadata": {
                    "resourceVersion": pod.metadata.resource_version,
                    "annotations": {key: revision},
                }});
                match pods
                    .patch(&pod_name, &PatchParams::default(), &Patch::Merge(&patch))
                    .await
                {
                    Ok(_) => {}
                    Err(kube::Error::Api(error)) if matches!(error.code, 404 | 409) => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(())
    }

    pub(super) async fn apply_mapping_tls_filter(
        &self,
        workspace: &Workspace,
        mapping: &PortMapping,
    ) -> Result<Option<String>, ReconcileError> {
        let Some(filter) = self.builder.port_mapping_tls_filter(workspace, mapping)? else {
            return Ok(None);
        };
        let name = filter
            .metadata
            .name
            .as_deref()
            .expect("generated filter is named");
        let api = self.mapping_filters();
        verify_existing(&api, name, &self.builder, workspace.id).await?;
        api.patch(
            name,
            &PatchParams::apply(FIELD_MANAGER),
            &Patch::Apply(&filter),
        )
        .await?;
        Ok(Some(name.to_owned()))
    }

    pub(super) async fn prune_mapping_tls_filters(
        &self,
        workspace: &Workspace,
        desired: &BTreeSet<String>,
    ) -> Result<(), ReconcileError> {
        let api = self.mapping_filters();
        let selector = format!(
            "{OWNER_INSTALLATION_LABEL}={},{WORKSPACE_ID_LABEL}={},{}",
            self.builder.installation_id,
            workspace.id,
            port_mappings::PORT_MAPPING_ID_LABEL,
        );
        for filter in api.list(&ListParams::default().labels(&selector)).await? {
            if let Some(name) = filter.metadata.name.as_deref()
                && !desired.contains(name)
            {
                self.builder
                    .verify_delete_ownership(&filter.metadata, workspace.id)?;
                let uid = filter
                    .metadata
                    .uid
                    .clone()
                    .ok_or(ReconcileError::MissingEnvoyFilterUid)?;
                api.delete(
                    name,
                    &DeleteParams::default().preconditions(Preconditions {
                        uid: Some(uid),
                        resource_version: filter.metadata.resource_version.clone(),
                    }),
                )
                .await?;
            }
        }
        Ok(())
    }

    fn mapping_filters(&self) -> Api<DynamicObject> {
        Api::namespaced_with(
            self.client.clone(),
            &self.builder.higress_namespace,
            &envoy_filter::api_resource(),
        )
    }
}
