use super::*;

pub(super) fn validate(
    workspace: &Workspace,
    binding: &WorkspaceHomeVolumeBinding,
    installation: &str,
    pods: &[Pod],
    workloads: &[StatefulSet],
) -> Result<(), ApiError> {
    let name = format!("w-{}", workspace.short_id);
    let workload = workloads
        .iter()
        .find(|workload| workload.metadata.name.as_deref() == Some(name.as_str()))
        .ok_or(StorageError::WorkspaceHomePvcUpdateConflict)?;
    let spec = workload
        .spec
        .as_ref()
        .ok_or(StorageError::WorkspaceHomePvcUpdateConflict)?;
    let pod_spec = spec
        .template
        .spec
        .as_ref()
        .ok_or(StorageError::WorkspaceHomePvcUpdateConflict)?;
    if !owned(&workload.metadata, workspace, installation)
        || workload.metadata.deletion_timestamp.is_some()
        || workload.metadata.uid.is_none()
        || spec.replicas != Some(1)
        || spec
            .volume_claim_templates
            .as_ref()
            .is_some_and(|claims| !claims.is_empty())
        || !uses_home(pod_spec, workspace, binding)
    {
        return Err(StorageError::WorkspaceHomePvcUpdateConflict.into());
    }
    let own_pods = pods
        .iter()
        .filter(|pod| owned(&pod.metadata, workspace, installation))
        .collect::<Vec<_>>();
    if own_pods.len() != 1 {
        return Err(StorageError::WorkspaceHomePvcUpdateConflict.into());
    }
    let pod = own_pods[0];
    let pod_spec = pod
        .spec
        .as_ref()
        .ok_or(StorageError::WorkspaceHomePvcUpdateConflict)?;
    let ready = pod.status.as_ref().is_some_and(|status| {
        status.phase.as_deref() == Some("Running")
            && status
                .conditions
                .iter()
                .flatten()
                .any(|condition| condition.type_ == "Ready" && condition.status == "True")
    });
    if pod.metadata.name.as_deref() != Some(format!("{name}-0").as_str())
        || pod.metadata.deletion_timestamp.is_some()
        || !pod.metadata.owner_references.iter().flatten().any(|owner| {
            owner.kind == "StatefulSet"
                && owner.controller == Some(true)
                && Some(owner.uid.as_str()) == workload.metadata.uid.as_deref()
        })
        || !ready
        || !uses_home(pod_spec, workspace, binding)
    {
        return Err(StorageError::WorkspaceHomePvcUpdateConflict.into());
    }
    if pods.iter().any(|other| {
        other.metadata.uid != pod.metadata.uid
            && other
                .spec
                .as_ref()
                .is_some_and(|spec| references(spec, &binding.claim_name))
    }) {
        return Err(StorageError::WorkspaceHomePvcInUse.into());
    }
    Ok(())
}

fn owned(
    metadata: &k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta,
    workspace: &Workspace,
    installation: &str,
) -> bool {
    metadata.labels.as_ref().is_some_and(|labels| {
        labels.get(WORKSPACE_ID_LABEL) == Some(&workspace.id.to_string())
            && labels.get(OWNER_INSTALLATION_LABEL).map(String::as_str) == Some(installation)
    })
}

fn uses_home(spec: &PodSpec, workspace: &Workspace, binding: &WorkspaceHomeVolumeBinding) -> bool {
    let home_volumes = spec
        .volumes
        .iter()
        .flatten()
        .filter(|volume| volume.name == "workspace-data")
        .collect::<Vec<_>>();
    if home_volumes.len() != 1
        || !home_volumes[0]
            .persistent_volume_claim
            .as_ref()
            .is_some_and(|claim| {
                claim.claim_name == binding.claim_name && claim.read_only != Some(true)
            })
    {
        return false;
    }
    let Some(container) = spec
        .containers
        .iter()
        .find(|container| container.name == "workspace")
    else {
        return false;
    };
    if !container.volume_mounts.iter().flatten().any(|mount| {
        mount.name == "workspace-data"
            && mount.mount_path == workspace.template.workspace_home
            && mount.sub_path.is_none()
            && mount.sub_path_expr.is_none()
            && mount.read_only != Some(true)
    }) {
        return false;
    }
    spec.containers
        .iter()
        .chain(spec.init_containers.iter().flatten())
        .all(|container| {
            container.volume_mounts.iter().flatten().all(|mount| {
                mount.mount_path != workspace.template.workspace_home
                    || mount.name == "workspace-data"
            })
        })
}
