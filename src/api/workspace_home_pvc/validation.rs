use k8s_openapi::api::{
    apps::v1::StatefulSet,
    core::v1::{PersistentVolumeClaim, Pod, PodSpec},
};
use kube::{Api, Client, api::ListParams};

use super::super::ApiError;
use crate::{
    kubernetes::{OWNER_INSTALLATION_LABEL, WORKSPACE_ID_LABEL, home_volume_capacity_bytes},
    storage::StorageError,
    workspaces::{Workspace, WorkspaceHomeVolumeBinding, WorkspaceState},
};

mod adoption;

pub(in crate::api) async fn validate(
    client: &Client,
    workspace: &Workspace,
    binding: &WorkspaceHomeVolumeBinding,
    installation: &str,
) -> Result<PersistentVolumeClaim, ApiError> {
    if binding.namespace != workspace.runtime.namespace() {
        return Err(StorageError::InvalidWorkspaceHomePvc.into());
    }
    let claim = Api::<PersistentVolumeClaim>::namespaced(client.clone(), &binding.namespace)
        .get_opt(&binding.claim_name)
        .await
        .map_err(ApiError::Kubernetes)?
        .ok_or(StorageError::WorkspaceHomePvcMismatch)?;
    validate_claim(&claim, workspace, binding, installation)?;
    let pods = Api::<Pod>::namespaced(client.clone(), &binding.namespace)
        .list(&ListParams::default())
        .await
        .map_err(ApiError::Kubernetes)?;
    let workloads = Api::<StatefulSet>::namespaced(client.clone(), &binding.namespace)
        .list(&ListParams::default())
        .await
        .map_err(ApiError::Kubernetes)?;
    for workload in &workloads.items {
        let Some(spec) = &workload.spec else { continue };
        let uses_claim = spec
            .template
            .spec
            .as_ref()
            .is_some_and(|spec| references(spec, &binding.claim_name))
            || spec.volume_claim_templates.iter().flatten().any(|claim| {
                let prefix = format!(
                    "{}-{}-",
                    claim.metadata.name.as_deref().unwrap_or_default(),
                    workload.metadata.name.as_deref().unwrap_or_default()
                );
                binding
                    .claim_name
                    .strip_prefix(&prefix)
                    .is_some_and(|ordinal| ordinal.parse::<u32>().is_ok())
            });
        if uses_claim
            && (workload.metadata.name.as_deref() != Some(&format!("w-{}", workspace.short_id))
                || workload
                    .metadata
                    .labels
                    .as_ref()
                    .and_then(|labels| labels.get(WORKSPACE_ID_LABEL))
                    != Some(&workspace.id.to_string())
                || workload
                    .metadata
                    .labels
                    .as_ref()
                    .and_then(|labels| labels.get(OWNER_INSTALLATION_LABEL))
                    .map(String::as_str)
                    != Some(installation))
        {
            return Err(StorageError::WorkspaceHomePvcInUse.into());
        }
    }
    if matches!(
        workspace.state,
        WorkspaceState::Ready | WorkspaceState::Failed
    ) {
        adoption::validate(
            workspace,
            binding,
            installation,
            &pods.items,
            &workloads.items,
        )?;
    } else if pods.items.iter().any(|pod| {
        pod.spec
            .as_ref()
            .is_some_and(|spec| references(spec, &binding.claim_name))
            || pod
                .metadata
                .labels
                .as_ref()
                .and_then(|labels| labels.get(WORKSPACE_ID_LABEL))
                == Some(&workspace.id.to_string())
    }) {
        return Err(StorageError::WorkspaceHomePvcInUse.into());
    }
    Ok(claim)
}

fn validate_claim(
    claim: &PersistentVolumeClaim,
    workspace: &Workspace,
    binding: &WorkspaceHomeVolumeBinding,
    installation: &str,
) -> Result<(), ApiError> {
    let status = claim
        .status
        .as_ref()
        .ok_or(StorageError::WorkspaceHomePvcMismatch)?;
    let spec = claim
        .spec
        .as_ref()
        .ok_or(StorageError::WorkspaceHomePvcMismatch)?;
    let capacity = status
        .capacity
        .as_ref()
        .and_then(|values| values.get("storage"));
    let requested = spec
        .resources
        .as_ref()
        .and_then(|resources| resources.requests.as_ref())
        .and_then(|values| values.get("storage"));
    if claim.metadata.namespace.as_deref() != Some(binding.namespace.as_str())
        || claim.metadata.uid.as_deref() != Some(binding.claim_uid.as_str())
        || claim.metadata.deletion_timestamp.is_some()
        || status.phase.as_deref() != Some("Bound")
        || spec.volume_name.as_deref().is_none_or(str::is_empty)
        || spec
            .volume_mode
            .as_deref()
            .is_some_and(|mode| mode != "Filesystem")
        || capacity.and_then(|quantity| home_volume_capacity_bytes(&quantity.0))
            != binding.capacity_gib.checked_mul(1 << 30)
        || requested.and_then(|quantity| home_volume_capacity_bytes(&quantity.0))
            != binding.capacity_gib.checked_mul(1 << 30)
    {
        return Err(StorageError::WorkspaceHomePvcMismatch.into());
    }
    if let Some(labels) = &claim.metadata.labels
        && (labels
            .get(WORKSPACE_ID_LABEL)
            .is_some_and(|owner| owner != &workspace.id.to_string())
            || labels
                .get(OWNER_INSTALLATION_LABEL)
                .is_some_and(|owner| owner != installation))
    {
        return Err(StorageError::WorkspaceHomePvcInUse.into());
    }
    Ok(())
}

fn references(spec: &PodSpec, claim_name: &str) -> bool {
    spec.volumes.iter().flatten().any(|volume| {
        volume
            .persistent_volume_claim
            .as_ref()
            .is_some_and(|claim| claim.claim_name == claim_name)
    })
}
