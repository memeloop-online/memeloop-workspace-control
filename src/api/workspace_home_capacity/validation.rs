use k8s_openapi::{
    api::{apps::v1::StatefulSet, core::v1::Pod},
    apimachinery::pkg::apis::meta::v1::ObjectMeta,
};
use kube::{Api, Client};

use super::super::{ApiError, workspace_home_pvc};
use crate::{
    kubernetes::{OWNER_INSTALLATION_LABEL, WORKSPACE_ID_LABEL},
    storage::StorageError,
    workspaces::{Workspace, WorkspaceHomeVolumeBinding},
};

pub(super) async fn validate(
    client: &Client,
    workspace: &Workspace,
    identity: &WorkspaceHomeVolumeBinding,
    installation: &str,
) -> Result<(), ApiError> {
    let claim =
        workspace_home_pvc::validation::validate(client, workspace, identity, installation).await?;
    if Api::<Pod>::namespaced(client.clone(), &identity.namespace)
        .get_opt(&format!("w-{}-0", workspace.short_id))
        .await
        .map_err(ApiError::Kubernetes)?
        .is_some()
    {
        return Err(StorageError::WorkspaceHomePvcInUse.into());
    }
    if claim.metadata.name.as_deref() != Some(identity.claim_name.as_str())
        || !owned(&claim.metadata, workspace, installation)
        || claim
            .metadata
            .owner_references
            .as_ref()
            .is_some_and(|owners| !owners.is_empty())
    {
        return Err(StorageError::WorkspaceHomePvcInUse.into());
    }
    let workload = Api::<StatefulSet>::namespaced(client.clone(), &identity.namespace)
        .get_opt(&format!("w-{}", workspace.short_id))
        .await
        .map_err(ApiError::Kubernetes)?
        .ok_or(StorageError::WorkspaceHomePvcUpdateConflict)?;
    let spec = workload
        .spec
        .as_ref()
        .ok_or(StorageError::WorkspaceHomePvcUpdateConflict)?;
    let home_templates = spec
        .volume_claim_templates
        .iter()
        .flatten()
        .filter(|claim| claim.metadata.name.as_deref() == Some("workspace-data"))
        .count();
    if !owned(&workload.metadata, workspace, installation)
        || workload.metadata.deletion_timestamp.is_some()
        || spec.replicas != Some(0)
        || spec
            .persistent_volume_claim_retention_policy
            .as_ref()
            .is_some_and(|policy| {
                policy
                    .when_deleted
                    .as_deref()
                    .is_some_and(|value| value != "Retain")
                    || policy
                        .when_scaled
                        .as_deref()
                        .is_some_and(|value| value != "Retain")
            })
        || home_templates != 1
        || spec.template.spec.as_ref().is_none_or(|pod| {
            pod.volumes
                .iter()
                .flatten()
                .any(|volume| volume.name == "workspace-data")
        })
    {
        return Err(StorageError::WorkspaceHomePvcUpdateConflict.into());
    }
    Ok(())
}

fn owned(metadata: &ObjectMeta, workspace: &Workspace, installation: &str) -> bool {
    metadata.namespace.as_deref() == Some(workspace.runtime.namespace())
        && metadata.labels.as_ref().is_some_and(|labels| {
            labels.get(WORKSPACE_ID_LABEL) == Some(&workspace.id.to_string())
                && labels.get(OWNER_INSTALLATION_LABEL).map(String::as_str) == Some(installation)
        })
}
