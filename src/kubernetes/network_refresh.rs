use std::collections::BTreeMap;

use k8s_openapi::api::networking::v1::NetworkPolicy;
use kube::{
    Api,
    api::{ListParams, Patch, PatchParams},
};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    storage::{Database, StorageError},
    workspace_runtime::WORKSPACE_NAMESPACE,
    workspaces::{Workspace, WorkspaceState},
};

use super::{
    BuildError, KubernetesCoordinator, ORGANIZATION_ID_LABEL, OWNER_INSTALLATION_LABEL,
    OwnershipError, WORKSPACE_ID_LABEL,
};

#[cfg(test)]
#[path = "network_refresh_tests.rs"]
mod tests;

#[derive(Debug, Error)]
pub enum NetworkRefreshError {
    #[error(transparent)]
    Kubernetes(#[from] kube::Error),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Build(#[from] BuildError),
    #[error(transparent)]
    Ownership(#[from] OwnershipError),
}

impl KubernetesCoordinator {
    /// Refresh existing policy objects after an operator configuration change.
    /// Never creates resources or modifies a Pod template.
    pub async fn refresh_network_policies(
        &self,
        database: &Database,
    ) -> Result<u64, NetworkRefreshError> {
        let api: Api<NetworkPolicy> = Api::namespaced(self.client.clone(), WORKSPACE_NAMESPACE);
        let selector = format!(
            "{OWNER_INSTALLATION_LABEL}={}",
            self.builder.installation_id
        );
        let mut cursor = String::new();
        let mut updated = 0;
        loop {
            let mut params = ListParams::default().labels(&selector).limit(100);
            if !cursor.is_empty() {
                params = params.continue_token(&cursor);
            }
            let page = api.list(&params).await?;
            for (organization, policies) in group_policies(page.items) {
                let ids: Vec<_> = policies.iter().map(|(id, _)| *id).collect();
                let workspaces = database.list_workspaces_by_ids(organization, &ids).await?;
                for workspace in workspaces {
                    if matches!(
                        workspace.state,
                        WorkspaceState::Deleting | WorkspaceState::Deleted
                    ) {
                        continue;
                    }
                    let mappings = if policies.iter().any(|(id, policy)| {
                        *id == workspace.id
                            && policy.metadata.labels.as_ref().is_some_and(|labels| {
                                labels.contains_key(super::port_mappings::PORT_MAPPING_ID_LABEL)
                            })
                    }) {
                        database.list_port_mappings(workspace.id).await?
                    } else {
                        Vec::new()
                    };
                    let desired = self.desired_network_policies(&workspace, &mappings)?;
                    for (_, policy) in policies.iter().filter(|(id, _)| *id == workspace.id) {
                        if let Some(target) = desired
                            .iter()
                            .find(|target| target.metadata.name == policy.metadata.name)
                        {
                            updated += u64::from(
                                self.patch_network_policy(&api, workspace.id, policy, target)
                                    .await?,
                            );
                        }
                    }
                }
            }
            cursor = page.metadata.continue_.unwrap_or_default();
            if cursor.is_empty() {
                return Ok(updated);
            }
        }
    }

    fn desired_network_policies(
        &self,
        workspace: &Workspace,
        mappings: &[crate::storage::PortMapping],
    ) -> Result<Vec<NetworkPolicy>, BuildError> {
        let mut policies = vec![self.builder.build(workspace)?.network_policy];
        for mapping in mappings {
            if mapping.workspace_id == workspace.id
                && mapping.organization_id == workspace.organization_id
                && let Some((_, _, policy)) =
                    self.builder.port_mapping_resources(workspace, mapping)?
            {
                policies.push(policy);
            }
        }
        Ok(policies)
    }

    #[cfg(test)]
    async fn refresh_policy(
        &self,
        api: &Api<NetworkPolicy>,
        workspace: &Workspace,
        policy: &NetworkPolicy,
    ) -> Result<bool, NetworkRefreshError> {
        let desired = self.builder.build(workspace)?.network_policy;
        self.patch_network_policy(api, workspace.id, policy, &desired)
            .await
    }

    async fn patch_network_policy(
        &self,
        api: &Api<NetworkPolicy>,
        workspace_id: Uuid,
        policy: &NetworkPolicy,
        desired: &NetworkPolicy,
    ) -> Result<bool, NetworkRefreshError> {
        let mut current = policy.clone();
        for attempt in 0..3 {
            if current.metadata.deletion_timestamp.is_some()
                || current.metadata.name != desired.metadata.name
            {
                return Ok(false);
            }
            self.builder
                .verify_delete_ownership(&current.metadata, workspace_id)?;
            let mapping_label = super::port_mappings::PORT_MAPPING_ID_LABEL;
            if let Some(expected) = desired
                .metadata
                .labels
                .as_ref()
                .and_then(|labels| labels.get(mapping_label))
            {
                let actual = current
                    .metadata
                    .labels
                    .as_ref()
                    .and_then(|labels| labels.get(mapping_label));
                if actual != Some(expected) {
                    return Err(OwnershipError::LabelMismatch {
                        key: mapping_label,
                        expected: expected.clone(),
                        actual: actual.cloned(),
                    }
                    .into());
                }
            }
            if current.spec == desired.spec {
                return Ok(false);
            }
            let Some(name) = current.metadata.name.as_deref() else {
                return Ok(false);
            };
            let patch = serde_json::json!({
                "metadata": {"resourceVersion": current.metadata.resource_version},
                "spec": desired.spec,
            });
            let params = PatchParams {
                field_manager: Some("memeloop-workspace-control".to_owned()),
                ..PatchParams::default()
            };
            match api.patch(name, &params, &Patch::Merge(&patch)).await {
                Ok(_) => return Ok(true),
                Err(kube::Error::Api(error)) if error.code == 404 => return Ok(false),
                Err(kube::Error::Api(error)) if error.code == 409 && attempt < 2 => {
                    let Some(latest) = api.get_opt(name).await? else {
                        return Ok(false);
                    };
                    current = latest;
                }
                Err(error) => return Err(error.into()),
            }
        }
        unreachable!("the final patch attempt returns either success or an error")
    }
}

fn group_policies(policies: Vec<NetworkPolicy>) -> BTreeMap<Uuid, Vec<(Uuid, NetworkPolicy)>> {
    let mut groups = BTreeMap::<Uuid, Vec<(Uuid, NetworkPolicy)>>::new();
    for policy in policies {
        if policy.metadata.deletion_timestamp.is_some() {
            continue;
        }
        let Some(labels) = &policy.metadata.labels else {
            continue;
        };
        let ids = labels
            .get(ORGANIZATION_ID_LABEL)
            .zip(labels.get(WORKSPACE_ID_LABEL));
        if let Some((Ok(organization), Ok(workspace))) =
            ids.map(|(org, workspace)| (Uuid::parse_str(org), Uuid::parse_str(workspace)))
        {
            groups
                .entry(organization)
                .or_default()
                .push((workspace, policy));
        }
    }
    groups
}
