use std::collections::BTreeMap;

use k8s_openapi::api::{
    apps::v1::StatefulSet,
    core::v1::{ConfigMap, ConfigMapVolumeSource, KeyToPath, Volume, VolumeMount},
};
use kube::Api;

use crate::workspaces::{Workspace, WorkspaceState};

use super::{DesiredResources, KubernetesCoordinator, ReconcileError, observe_apply_stage};

const GATE_ANNOTATION: &str = "workspace.memeloop.dev/startup-gate";
const GATE_VOLUME: &str = "workspace-startup-gate";
const FILE_ENCODING_ANNOTATION: &str = "workspace.memeloop.dev/file-encoding";
const FILE_ENCODING: &str = "dictionary-v1";

pub(super) struct StartupPlan {
    pub workload: StatefulSet,
    pub existing: Option<StatefulSet>,
    pub gate: Option<ConfigMap>,
    pub overlap: bool,
    pub compress_files: bool,
}

impl KubernetesCoordinator {
    pub(super) async fn prepare_startup_plan(
        &self,
        workspace: &Workspace,
        desired: &DesiredResources,
    ) -> Result<StartupPlan, ReconcileError> {
        let names = self.builder.runtime_names(workspace)?.resources;
        let namespace = workspace.runtime.namespace();
        let stateful_sets = Api::<StatefulSet>::namespaced(self.client.clone(), namespace);
        let existing = observe_apply_stage(
            workspace,
            "startup_workload_metadata",
            stateful_sets.get_opt(&names.stateful_set),
        )
        .await?;
        if let Some(existing) = &existing {
            self.builder
                .verify_delete_ownership(&existing.metadata, workspace.id)?;
        }
        let mut plan = StartupPlan {
            workload: desired.stateful_set.clone(),
            existing,
            gate: None,
            overlap: false,
            compress_files: false,
        };
        let annotations = plan
            .existing
            .as_ref()
            .and_then(|object| object.metadata.annotations.as_ref());
        let previous_gate = annotations.and_then(|a| a.get(GATE_ANNOTATION));
        let is_new = plan.existing.is_none() && workspace.state == WorkspaceState::Provisioning;
        if !is_new && previous_gate.is_none() {
            return Ok(plan);
        }
        let previous_encoding = annotations.and_then(|a| a.get(FILE_ENCODING_ANNOTATION));
        if previous_encoding.is_some_and(|value| value != FILE_ENCODING) {
            return Err(ReconcileError::RuntimeIdentityMismatch);
        }
        // Existing bootstrap monitor shells do not reload their functions when their script
        // projection changes. Only newly created Pods may opt in to the new transport.
        plan.compress_files = is_new || previous_encoding.is_some();
        if plan.compress_files {
            plan.workload
                .metadata
                .annotations
                .get_or_insert_default()
                .insert(
                    FILE_ENCODING_ANNOTATION.to_owned(),
                    FILE_ENCODING.to_owned(),
                );
        }
        let name = names.startup_gate_config_map();
        if previous_gate.is_some_and(|value| value != &name) {
            return Err(ReconcileError::RuntimeIdentityMismatch);
        }
        let config_maps = Api::<ConfigMap>::namespaced(self.client.clone(), namespace);
        let approved = observe_apply_stage(
            workspace,
            "startup_approval_read",
            config_maps.get_opt(&name),
        )
        .await?;
        if let Some(approved) = &approved {
            self.builder
                .verify_delete_ownership(&approved.metadata, workspace.id)?;
            if approved.immutable != Some(true)
                || approved
                    .data
                    .as_ref()
                    .and_then(|d| d.get("approved"))
                    .map(String::as_str)
                    != Some("true")
            {
                return Err(ReconcileError::RuntimeIdentityMismatch);
            }
        }
        // Do not turn a previously approved, missing StatefulSet into another early creation.
        // Its inputs may belong to a later update; the ordinary ordering is required then.
        plan.overlap = approved.is_none()
            && matches!(
                workspace.state,
                WorkspaceState::Provisioning | WorkspaceState::Starting
            );
        let mut metadata = desired.workspace_config.metadata.clone();
        metadata.name = Some(name.clone());
        plan.gate = Some(ConfigMap {
            metadata,
            immutable: Some(true),
            data: Some(BTreeMap::from([("approved".to_owned(), "true".to_owned())])),
            ..ConfigMap::default()
        });
        add_gate(&mut plan.workload, &name)?;
        Ok(plan)
    }

    pub(super) async fn apply_startup_prerequisites(
        &self,
        namespace: &str,
        workspace: &Workspace,
        desired: &DesiredResources,
        startup: &StartupPlan,
    ) -> Result<(), ReconcileError> {
        tokio::try_join!(
            observe_apply_stage(
                workspace,
                "injections",
                self.apply_injections(workspace, &desired.injections, startup.compress_files)
            ),
            observe_apply_stage(
                workspace,
                "workspace_identity",
                self.apply_workspace_identity(namespace, workspace, desired)
            ),
            observe_apply_stage(
                workspace,
                "services",
                self.apply_workspace_services(namespace, workspace, desired)
            ),
            async {
                // A named ServiceAccount must exist before Pod admission. Establish the
                // isolation policy too, before a Pod can acquire a network namespace.
                tokio::try_join!(
                    observe_apply_stage(
                        workspace,
                        "access_identity",
                        self.apply_access_identity(namespace, workspace, desired)
                    ),
                    observe_apply_stage(
                        workspace,
                        "network_policy",
                        self.apply_network_policy(namespace, workspace, desired)
                    ),
                )?;
                if startup.overlap {
                    // Other projected inputs and Services may still be pending. Every
                    // container is blocked by the required, unapproved gate ConfigMap,
                    // while scheduling and volume preparation can already progress.
                    observe_apply_stage(
                        workspace,
                        "gated_workload",
                        Box::pin(self.apply_workload(
                            namespace,
                            workspace,
                            &startup.workload,
                            startup.existing.as_ref(),
                        )),
                    )
                    .await?;
                }
                Ok::<_, ReconcileError>(())
            },
        )?;
        Ok(())
    }
}

fn add_gate(workload: &mut StatefulSet, name: &str) -> Result<(), ReconcileError> {
    workload
        .metadata
        .annotations
        .get_or_insert_default()
        .insert(GATE_ANNOTATION.to_owned(), name.to_owned());
    let pod = workload
        .spec
        .as_mut()
        .and_then(|s| s.template.spec.as_mut())
        .ok_or(ReconcileError::RuntimeIdentityMismatch)?;
    pod.volumes.get_or_insert_default().push(Volume {
        name: GATE_VOLUME.to_owned(),
        config_map: Some(ConfigMapVolumeSource {
            name: name.to_owned(),
            optional: Some(false),
            items: Some(vec![KeyToPath {
                key: "approved".to_owned(),
                path: "approved".to_owned(),
                mode: Some(0o444),
            }]),
            ..ConfigMapVolumeSource::default()
        }),
        ..Volume::default()
    });
    for container in pod
        .containers
        .iter_mut()
        .chain(pod.init_containers.iter_mut().flatten())
    {
        container
            .volume_mounts
            .get_or_insert_default()
            .push(VolumeMount {
                name: GATE_VOLUME.to_owned(),
                mount_path: "/var/run/workspace-startup-gate".to_owned(),
                read_only: Some(true),
                ..VolumeMount::default()
            });
    }
    Ok(())
}
