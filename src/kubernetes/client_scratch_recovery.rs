use k8s_openapi::api::{
    apps::v1::StatefulSet,
    core::v1::{ContainerStatus, PersistentVolume, PersistentVolumeClaim, Pod, Volume},
    storage::v1::StorageClass,
};
use kube::{
    Api,
    api::{DeleteParams, Preconditions},
};

use super::{
    KubernetesCoordinator, ReconcileError, current_workspace_pod, observed_workspace_replicas,
};
use crate::{
    workspace_runtime::WorkspaceResourceNames,
    workspaces::{Workspace, WorkspaceState},
};

impl KubernetesCoordinator {
    /// A recreated WFFC claim cannot schedule its already assigned consumer.
    /// Replace only an inactive Pod; preserve Home and all claims and volumes.
    pub(crate) async fn recover_scratch_scheduling(
        &self,
        workspace: &Workspace,
    ) -> Result<(), ReconcileError> {
        if !matches!(
            workspace.state,
            WorkspaceState::Provisioning
                | WorkspaceState::Starting
                | WorkspaceState::Restarting
                | WorkspaceState::Ready
        ) || workspace.scratch_volume_binding.is_some()
        {
            return Ok(());
        }
        let names = self.builder.runtime_names(workspace)?.resources;
        let namespace = workspace.runtime.namespace();
        let pods = Api::<Pod>::namespaced(self.client.clone(), namespace);
        let Some(pod) = pods.get_opt(&names.pod_ordinal_zero()).await? else {
            return Ok(());
        };
        self.builder
            .verify_delete_ownership(&pod.metadata, workspace.id)?;
        if !inactive_scheduled_pod(&pod) {
            return Ok(());
        }
        let (Some(pod_uid), Some(pod_version)) = (
            pod.metadata.uid.as_ref(),
            pod.metadata.resource_version.as_ref(),
        ) else {
            return Ok(());
        };
        let Some(set) = Api::<StatefulSet>::namespaced(self.client.clone(), namespace)
            .get_opt(&names.stateful_set)
            .await?
        else {
            return Ok(());
        };
        self.builder
            .verify_delete_ownership(&set.metadata, workspace.id)?;
        if !observed_workspace_replicas(&set, workspace, 1)
            || !current_workspace_pod(&set, &pod, workspace, &names.stateful_set)
        {
            return Ok(());
        }
        let Some((scratch_name, ephemeral)) = scratch_claim_name(&pod, &names) else {
            return Ok(());
        };
        let claims = Api::<PersistentVolumeClaim>::namespaced(self.client.clone(), namespace);
        let Some(claim) = claims.get_opt(&scratch_name).await? else {
            return Ok(());
        };
        self.builder
            .verify_delete_ownership(&claim.metadata, workspace.id)?;
        if !empty_recreated_scratch(&claim, &pod, ephemeral) {
            return Ok(());
        }
        let Some(class_name) = claim
            .spec
            .as_ref()
            .and_then(|spec| spec.storage_class_name.as_deref())
        else {
            return Ok(());
        };
        let Some(class) = Api::<StorageClass>::all(self.client.clone())
            .get_opt(class_name)
            .await?
        else {
            return Ok(());
        };
        if class.volume_binding_mode.as_deref() != Some("WaitForFirstConsumer")
            || !self
                .recovery_home_is_retained(workspace, &pod, &set, &names)
                .await?
        {
            return Ok(());
        }
        match pods
            .delete(
                &names.pod_ordinal_zero(),
                &DeleteParams::default().preconditions(Preconditions {
                    uid: Some(pod_uid.clone()),
                    resource_version: Some(pod_version.clone()),
                }),
            )
            .await
        {
            Ok(_) => {
                tracing::info!(workspace_id = %workspace.id, generation = workspace.generation,
                    pod_uid, scratch_claim = %scratch_name,
                    "replacing inactive scheduled Pod: newer Pending WFFC scratch claim has no selected node");
                Ok(())
            }
            // A changed/replaced Pod is authoritative; use the existing job retry.
            Err(kube::Error::Api(error)) if matches!(error.code, 404 | 409) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    async fn recovery_home_is_retained(
        &self,
        workspace: &Workspace,
        pod: &Pod,
        set: &StatefulSet,
        names: &WorkspaceResourceNames,
    ) -> Result<bool, ReconcileError> {
        self.verify_home_volume_binding(workspace).await?;
        let home_name = workspace.home_volume_binding.as_ref().map_or_else(
            || names.data_pvc_ordinal_zero(),
            |binding| binding.claim_name.clone(),
        );
        if !recovery_home_reference(workspace, pod, set, names, &home_name) {
            return Ok(false);
        }
        let claims = Api::<PersistentVolumeClaim>::namespaced(
            self.client.clone(),
            workspace.runtime.namespace(),
        );
        let Some(home) = claims.get_opt(&home_name).await? else {
            return Ok(false);
        };
        if workspace.home_volume_binding.is_none() {
            self.builder
                .verify_delete_ownership(&home.metadata, workspace.id)?;
            if home
                .metadata
                .creation_timestamp
                .as_ref()
                .zip(pod.metadata.creation_timestamp.as_ref())
                .is_none_or(|(home_created, pod_created)| home_created.0 > pod_created.0)
            {
                return Ok(false);
            }
        }
        if home.metadata.uid.is_none()
            || home.metadata.deletion_timestamp.is_some()
            || home
                .metadata
                .owner_references
                .as_ref()
                .is_some_and(|owners| owners.iter().any(|owner| owner.kind == "Pod"))
            || home
                .status
                .as_ref()
                .is_none_or(|status| status.phase.as_deref() != Some("Bound"))
            || workspace
                .home_volume_binding
                .as_ref()
                .is_some_and(|binding| {
                    home.metadata.uid.as_deref() != Some(binding.claim_uid.as_str())
                })
        {
            return Ok(false);
        }
        let Some(home_volume) = home
            .spec
            .as_ref()
            .and_then(|spec| spec.volume_name.as_deref())
        else {
            return Ok(false);
        };
        let Some(volume) = Api::<PersistentVolume>::all(self.client.clone())
            .get_opt(home_volume)
            .await?
        else {
            return Ok(false);
        };
        Ok(volume.metadata.uid.is_some()
            && volume.metadata.deletion_timestamp.is_none()
            && volume
                .status
                .as_ref()
                .is_some_and(|status| status.phase.as_deref() == Some("Bound"))
            && volume
                .spec
                .as_ref()
                .and_then(|spec| spec.claim_ref.as_ref())
                .is_some_and(|reference| {
                    reference.name.as_deref() == Some(home_name.as_str())
                        && reference.namespace.as_deref() == Some(workspace.runtime.namespace())
                        && reference.uid == home.metadata.uid
                }))
    }
}

fn scratch_claim_name(pod: &Pod, names: &WorkspaceResourceNames) -> Option<(String, bool)> {
    let scratch = pod
        .spec
        .as_ref()?
        .volumes
        .as_ref()?
        .iter()
        .find(|volume| volume.name == "workspace-scratch")?;
    if scratch.ephemeral.is_some() && scratch.persistent_volume_claim.is_none() {
        Some((
            format!("{}-workspace-scratch", names.pod_ordinal_zero()),
            true,
        ))
    } else if scratch.ephemeral.is_none()
        && scratch
            .persistent_volume_claim
            .as_ref()
            .is_some_and(|claim| {
                claim.claim_name == names.scratch_pvc() && claim.read_only != Some(true)
            })
    {
        Some((names.scratch_pvc(), false))
    } else {
        None
    }
}

fn empty_recreated_scratch(claim: &PersistentVolumeClaim, pod: &Pod, ephemeral: bool) -> bool {
    let (Some(spec), Some(created), Some(pod_created)) = (
        claim.spec.as_ref(),
        claim.metadata.creation_timestamp.as_ref(),
        pod.metadata.creation_timestamp.as_ref(),
    ) else {
        return false;
    };
    let owners = claim
        .metadata
        .owner_references
        .as_deref()
        .unwrap_or_default();
    let owned = if ephemeral {
        owners.len() == 1
            && owners[0].controller == Some(true)
            && owners[0].api_version == "v1"
            && owners[0].kind == "Pod"
            && Some(owners[0].name.as_str()) == pod.metadata.name.as_deref()
            && Some(owners[0].uid.as_str()) == pod.metadata.uid.as_deref()
    } else {
        owners.is_empty()
    };
    owned
        && created.0 > pod_created.0
        && claim.metadata.uid.is_some()
        && claim.metadata.namespace == pod.metadata.namespace
        && claim.metadata.deletion_timestamp.is_none()
        && claim.metadata.labels.as_ref().is_some_and(|labels| {
            labels
                .get(super::super::STORAGE_ROLE_LABEL)
                .map(String::as_str)
                == Some("temporary")
        })
        && claim
            .metadata
            .annotations
            .as_ref()
            .is_none_or(|annotations| {
                [
                    "volume.kubernetes.io/selected-node",
                    "pv.kubernetes.io/bind-completed",
                    "pv.kubernetes.io/bound-by-controller",
                ]
                .iter()
                .all(|key| !annotations.contains_key(*key))
            })
        && spec.volume_name.is_none()
        && spec.data_source.is_none()
        && spec.data_source_ref.is_none()
        && claim.status.as_ref().is_some_and(|status| {
            status.phase.as_deref() == Some("Pending") && status.capacity.is_none()
        })
}

fn recovery_home_reference(
    workspace: &Workspace,
    pod: &Pod,
    set: &StatefulSet,
    names: &WorkspaceResourceNames,
    home: &str,
) -> bool {
    let matches = |volumes: Option<&Vec<Volume>>| {
        volumes.is_some_and(|volumes| {
            volumes.iter().any(|volume| {
                volume.name == names.data_claim_template
                    && volume
                        .persistent_volume_claim
                        .as_ref()
                        .is_some_and(|claim| claim.claim_name == home)
            })
        })
    };
    matches(pod.spec.as_ref().and_then(|spec| spec.volumes.as_ref()))
        && set.spec.as_ref().is_some_and(|spec| {
            if spec
                .template
                .spec
                .as_ref()
                .and_then(|spec| spec.volumes.as_ref())
                .is_some_and(|volumes| {
                    volumes
                        .iter()
                        .any(|volume| volume.name == names.data_claim_template)
                })
            {
                matches(
                    spec.template
                        .spec
                        .as_ref()
                        .and_then(|spec| spec.volumes.as_ref()),
                )
            } else {
                workspace.home_volume_binding.is_none()
                    && spec.volume_claim_templates.as_ref().is_some_and(|claims| {
                        claims.iter().any(|claim| {
                            claim.metadata.name.as_deref()
                                == Some(names.data_claim_template.as_str())
                        })
                    })
            }
        })
}

fn inactive_scheduled_pod(pod: &Pod) -> bool {
    let (Some(spec), Some(status)) = (pod.spec.as_ref(), pod.status.as_ref()) else {
        return false;
    };
    let terminated = |statuses: Option<&Vec<ContainerStatus>>, names: Vec<&str>| {
        statuses.into_iter().flatten().all(|status| {
            status.state.as_ref().is_some_and(|state| {
                state.terminated.is_some() && state.running.is_none() && state.waiting.is_none()
            })
        }) && names.iter().all(|name| {
            statuses.is_some_and(|statuses| {
                statuses.iter().any(|status| {
                    status.name == *name
                        && status.state.as_ref().is_some_and(|state| {
                            state.terminated.is_some()
                                && state.running.is_none()
                                && state.waiting.is_none()
                        })
                })
            })
        })
    };
    pod.metadata.deletion_timestamp.is_none()
        && spec
            .node_name
            .as_deref()
            .is_some_and(|name| !name.is_empty())
        && status.phase.as_deref() == Some("Running")
        && status.pod_ip.as_deref().is_none_or(str::is_empty)
        && status.conditions.as_ref().is_some_and(|conditions| {
            conditions
                .iter()
                .any(|condition| condition.type_ == "Ready" && condition.status == "False")
        })
        && !spec.containers.is_empty()
        && terminated(
            status.container_statuses.as_ref(),
            spec.containers
                .iter()
                .map(|container| container.name.as_str())
                .collect(),
        )
        && terminated(
            status.init_container_statuses.as_ref(),
            spec.init_containers
                .iter()
                .flatten()
                .map(|container| container.name.as_str())
                .collect(),
        )
        && terminated(
            status.ephemeral_container_statuses.as_ref(),
            spec.ephemeral_containers
                .iter()
                .flatten()
                .map(|container| container.name.as_str())
                .collect(),
        )
}
