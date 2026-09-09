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
                    for (_, policy) in policies.iter().filter(|(id, _)| *id == workspace.id) {
                        updated += u64::from(self.refresh_policy(&api, &workspace, policy).await?);
                    }
                }
            }
            cursor = page.metadata.continue_.unwrap_or_default();
            if cursor.is_empty() {
                return Ok(updated);
            }
        }
    }

    async fn refresh_policy(
        &self,
        api: &Api<NetworkPolicy>,
        workspace: &Workspace,
        policy: &NetworkPolicy,
    ) -> Result<bool, NetworkRefreshError> {
        let desired = self.builder.build(workspace)?.network_policy;
        let mut current = policy.clone();
        for attempt in 0..3 {
            if current.metadata.deletion_timestamp.is_some()
                || current.metadata.name != desired.metadata.name
            {
                return Ok(false);
            }
            self.builder
                .verify_delete_ownership(&current.metadata, workspace.id)?;
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
