use k8s_openapi::api::{
    apps::v1::StatefulSet,
    core::v1::{Container, Pod, ResourceRequirements},
};
use kube::{
    Api,
    api::{Patch, PatchParams},
};
use serde_json::{Value, json};

use super::{
    FIELD_MANAGER, KubernetesCoordinator, ReconcileError, current_workspace_pod,
    home_volume_capacity_bytes, observed_workspace_generation, workspace_connection_ready,
};
use crate::workspaces::Workspace;

// StatefulSet CurrentRevision can stay at the first revision indefinitely
// under OnDelete. Record the exact live revision that this resize preserves.
const RETAINED_REVISION: &str = "workspace.memeloop.dev/memory-resize-retained-revision";

pub(super) fn retained_memory_revision(set: &StatefulSet) -> Option<&str> {
    if set
        .spec
        .as_ref()?
        .update_strategy
        .as_ref()?
        .type_
        .as_deref()
        != Some("OnDelete")
    {
        return None;
    }
    set.metadata
        .annotations
        .as_ref()?
        .get(RETAINED_REVISION)
        .map(String::as_str)
}

pub(crate) enum MemoryResize {
    Applied,
    Pending,
    Rejected(&'static str),
}

impl KubernetesCoordinator {
    pub(crate) async fn increase_workspace_memory(
        &self,
        workspace: &Workspace,
    ) -> Result<MemoryResize, ReconcileError> {
        let names = self.builder.runtime_names(workspace)?.resources;
        let sets =
            Api::<StatefulSet>::namespaced(self.client.clone(), workspace.runtime.namespace());
        let pods = Api::<Pod>::namespaced(self.client.clone(), workspace.runtime.namespace());
        let name = names.pod_ordinal_zero();
        let (set, pod) = tokio::try_join!(sets.get_opt(&names.stateful_set), pods.get_opt(&name))?;
        let (Some(set), Some(pod)) = (set, pod) else {
            return Ok(MemoryResize::Pending);
        };
        self.builder
            .verify_delete_ownership(&set.metadata, workspace.id)?;
        self.builder
            .verify_delete_ownership(&pod.metadata, workspace.id)?;
        let retained = retained_memory_revision(&set);
        let Some(generation) = observed_workspace_generation(&set, 1, retained.is_some()) else {
            return Ok(MemoryResize::Pending);
        };
        if generation > workspace.generation
            || !current_workspace_pod(&set, &pod, generation, &names.stateful_set, retained)
            || !workspace_connection_ready(&pod)
        {
            return Ok(MemoryResize::Pending);
        }
        let Some((pod_uid, pod_version, revision)) = pod
            .metadata
            .uid
            .as_ref()
            .zip(pod.metadata.resource_version.as_ref())
            .zip(
                pod.metadata
                    .labels
                    .as_ref()
                    .and_then(|labels| labels.get("controller-revision-hash")),
            )
            .map(|((uid, version), revision)| (uid, version, revision))
        else {
            return Ok(MemoryResize::Rejected("runtime_changed"));
        };
        let Some((set_uid, set_version)) = set
            .metadata
            .uid
            .as_ref()
            .zip(set.metadata.resource_version.as_ref())
        else {
            return Ok(MemoryResize::Rejected("runtime_changed"));
        };
        let Some(set_container) = set
            .spec
            .as_ref()
            .and_then(|spec| spec.template.spec.as_ref())
            .and_then(|spec| {
                spec.containers
                    .iter()
                    .find(|container| container.name == "workspace")
            })
        else {
            return Ok(MemoryResize::Rejected("runtime_changed"));
        };
        let Some(container) = pod.spec.as_ref().and_then(|spec| {
            spec.containers
                .iter()
                .find(|container| container.name == "workspace")
        }) else {
            return Ok(MemoryResize::Rejected("runtime_changed"));
        };
        let Some((wanted, replacement)) =
            container_limit(container).zip(container_limit(set_container))
        else {
            return Ok(MemoryResize::Rejected("runtime_changed"));
        };
        let limit = format!("{}Mi", workspace.template.resources.memory_mib);
        let Some(target) = home_volume_capacity_bytes(&limit) else {
            return Ok(MemoryResize::Rejected("rejected"));
        };
        if wanted > target || replacement > target {
            // This endpoint can only ever increase, including after a retry.
            return Ok(MemoryResize::Rejected("runtime_changed"));
        }

        let patch = pod_resize_patch(pod_uid, pod_version, &limit);
        let params = PatchParams::apply(FIELD_MANAGER);
        if wanted < target {
            // The actual endpoint checks feature availability, authorization,
            // admission and QoS before changing the replacement template.
            if container.resize_policy.as_ref().is_some_and(|policies| {
                policies.iter().any(|policy| {
                    policy.resource_name == "memory" && policy.restart_policy != "NotRequired"
                })
            }) {
                return Ok(MemoryResize::Rejected("unsupported"));
            }
            let dry_run = pods
                .patch_subresource("resize", &name, &params.clone().dry_run(), &patch)
                .await;
            if let Err(error) = dry_run {
                return resize_api_error(error, true, true);
            }
        }
        if replacement < target {
            // Save the next Pod's resources first; OnDelete prevents the
            // StatefulSet controller from recreating this running Pod.
            let patch = Patch::Strategic(json!({
                "metadata": {
                    "uid": set_uid,
                    "resourceVersion": set_version,
                    "annotations": {(RETAINED_REVISION): revision},
                },
                "spec": {
                    "updateStrategy": {"type": "OnDelete", "rollingUpdate": null},
                    "template": {"spec": {"containers": [workspace_memory_patch(&limit)]}},
                },
            }));
            if let Err(error) = sets.patch(&names.stateful_set, &params, &patch).await {
                return resize_api_error(error, false, false);
            }
        }
        if wanted < target {
            match pods
                .patch_subresource("resize", &name, &params, &patch)
                .await
            {
                Ok(pod) => return Ok(observed_resize(&pod, target)),
                Err(error) => return resize_api_error(error, false, true),
            }
        }
        Ok(observed_resize(&pod, target))
    }
}

fn workspace_memory_patch(limit: &str) -> Value {
    json!({"name": "workspace", "resources": {"limits": {"memory": limit}}})
}

fn pod_resize_patch(uid: &str, version: &str, limit: &str) -> Patch<Value> {
    Patch::Strategic(json!({
        "metadata": {"uid": uid, "resourceVersion": version},
        "spec": {"containers": [workspace_memory_patch(limit)]},
    }))
}

fn memory_limit(resources: &ResourceRequirements) -> Option<u64> {
    home_volume_capacity_bytes(&resources.limits.as_ref()?.get("memory")?.0)
}

fn container_limit(container: &Container) -> Option<u64> {
    memory_limit(container.resources.as_ref()?)
}

fn observed_resize(pod: &Pod, target: u64) -> MemoryResize {
    let Some(status) = &pod.status else {
        return MemoryResize::Pending;
    };
    if status
        .container_statuses
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find(|container| container.name == "workspace")
        .and_then(|container| container.resources.as_ref())
        .and_then(memory_limit)
        == Some(target)
    {
        MemoryResize::Applied
    } else if status
        .conditions
        .as_deref()
        .unwrap_or_default()
        .iter()
        .any(|condition| {
            condition.type_ == "PodResizePending"
                && condition.status == "True"
                && condition.reason.as_deref() == Some("Infeasible")
                && condition.observed_generation == pod.metadata.generation
        })
    {
        MemoryResize::Rejected("infeasible")
    } else {
        MemoryResize::Pending
    }
}

fn resize_api_error(
    error: kube::Error,
    dry_run: bool,
    pod: bool,
) -> Result<MemoryResize, ReconcileError> {
    let kube::Error::Api(response) = &error else {
        return Err(error.into());
    };
    match response.code {
        409 => Ok(MemoryResize::Pending),
        // A named Pod can disappear between the read and patch. A missing
        // endpoint normally has API reason NotFound but no resource details.
        404 if dry_run
            && response
                .message
                .contains("the server could not find the requested resource") =>
        {
            Ok(MemoryResize::Rejected("unsupported"))
        }
        404 => Ok(MemoryResize::Pending),
        405 | 501 if pod => Ok(MemoryResize::Rejected("unsupported")),
        401 | 403 => Ok(MemoryResize::Rejected("forbidden")),
        400 | 422 => Ok(MemoryResize::Rejected("rejected")),
        _ => Err(error.into()),
    }
}
