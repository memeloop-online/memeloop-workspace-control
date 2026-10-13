use crate::{
    jobs::JobHandlerError,
    kubernetes::ShrinkPodStage,
    storage::{
        ClaimedJob, NewJob, ShrinkWorkspaceUpdate, StorageError, StorageShrinkOperation,
        StorageShrinkPhase, StorageShrinkVolume,
    },
    workspaces::{Workspace, WorkspaceState},
};

use super::{WorkspaceReconcileHandler, job_error, unix_timestamp};

impl WorkspaceReconcileHandler {
    pub(super) async fn handle_storage_shrink(
        &self,
        job: &ClaimedJob,
    ) -> Result<(), JobHandlerError> {
        let workspace_id = job
            .workspace_id
            .ok_or_else(|| job_error("storage shrink job has no workspace"))?;
        let Some(mut operation) = self
            .database
            .latest_storage_shrink(workspace_id)
            .await
            .map_err(job_error)?
        else {
            return Ok(());
        };
        if !operation.phase.active()
            || job.payload.get("operation_id").and_then(|id| id.as_str())
                != Some(operation.id.to_string().as_str())
        {
            return Ok(());
        }
        let workspace = self
            .database
            .get_workspace(workspace_id)
            .await
            .map_err(job_error)?;
        let now = unix_timestamp()?;
        if operation.phase == StorageShrinkPhase::Queued && operation.error.is_some() {
            return self
                .finish_preflight_failure(&workspace, &mut operation, now)
                .await;
        }
        let result = if now.saturating_sub(operation.updated_at) > 86_400
            && operation.phase != StorageShrinkPhase::RollingBack
        {
            Err(job_error(
                "storage migration phase exceeded its 24-hour deadline",
            ))
        } else {
            self.advance_storage_shrink(&workspace, &mut operation, job, now)
                .await
        };
        match result {
            Err(JobHandlerError::Failed(error))
                if operation.phase == StorageShrinkPhase::Reclaiming =>
            {
                operation.error = Some(error);
                operation.phase = StorageShrinkPhase::Completed;
                operation.updated_at = now;
                self.database
                    .save_storage_shrink(&operation, ShrinkWorkspaceUpdate::None)
                    .await
                    .map_err(job_error)?;
                Ok(())
            }
            Err(JobHandlerError::Failed(error))
                if operation.phase != StorageShrinkPhase::RollingBack =>
            {
                operation.error = Some(error);
                operation.updated_at = now;
                if operation.phase == StorageShrinkPhase::Queued {
                    self.database
                        .save_storage_shrink(&operation, ShrinkWorkspaceUpdate::None)
                        .await
                        .map_err(job_error)?;
                    return self
                        .finish_preflight_failure(&workspace, &mut operation, now)
                        .await;
                }
                operation.phase = StorageShrinkPhase::RollingBack;
                self.database
                    .save_storage_shrink(&operation, ShrinkWorkspaceUpdate::Stopping)
                    .await
                    .map_err(job_error)?;
                pending("migration failed; stopping and preserving recovery volumes")
            }
            result => result,
        }
    }

    async fn advance_storage_shrink(
        &self,
        workspace: &Workspace,
        operation: &mut StorageShrinkOperation,
        job: &ClaimedJob,
        now: i64,
    ) -> Result<(), JobHandlerError> {
        use StorageShrinkPhase as Phase;
        match operation.phase {
            Phase::Queued => {
                self.coordinator
                    .discover_shrink_sources(workspace, operation)
                    .await
                    .map_err(job_error)?;
                self.database
                    .save_storage_shrink(operation, ShrinkWorkspaceUpdate::None)
                    .await
                    .map_err(job_error)?;
                if self
                    .run_shrink_pod(workspace, operation, "home", ShrinkPodStage::Preflight, now)
                    .await?
                    .is_none()
                {
                    return pending("checking migration tools before stopping the workspace");
                }
                if !self
                    .coordinator
                    .retain_shrink_sources(workspace, operation)
                    .await
                    .map_err(job_error)?
                {
                    return pending(
                        "waiting for the StatefulSet controller to observe Retain before removing GC owners",
                    );
                }
                operation.phase = Phase::Stopping;
                operation.updated_at = now;
                self.database
                    .save_storage_shrink(operation, ShrinkWorkspaceUpdate::Stopping)
                    .await
                    .map_err(job_error)?;
            }
            Phase::Stopping => {
                if !self
                    .coordinator
                    .stop_for_storage_shrink(workspace, operation)
                    .await
                    .map_err(job_error)?
                {
                    return pending("waiting for workspace Pod termination and volume exclusivity");
                }
                operation.phase = Phase::Measuring;
                operation.updated_at = now;
                self.database
                    .save_storage_shrink(operation, ShrinkWorkspaceUpdate::Stopped)
                    .await
                    .map_err(job_error)?;
            }
            Phase::Measuring => {
                for role in ["home", "temporary"] {
                    if volume(operation, role).target_gib.is_none()
                        || volume(operation, role).used_bytes.is_some()
                    {
                        continue;
                    }
                    let Some(usage) = self
                        .run_shrink_pod(workspace, operation, role, ShrinkPodStage::Measure, now)
                        .await?
                    else {
                        return pending("measuring offline storage usage");
                    };
                    let selected = volume_mut(operation, role);
                    selected.used_bytes = Some(usage.used_bytes);
                    selected.used_inodes = Some(usage.used_inodes);
                    if selected
                        .target_gib
                        .and_then(|target| target.checked_mul(1 << 30))
                        .is_none_or(|bytes| bytes <= usage.used_bytes)
                    {
                        return Err(job_error(format!(
                            "{role} target must be strictly larger than offline used bytes ({})",
                            usage.used_bytes
                        )));
                    }
                    operation.updated_at = now;
                    self.database
                        .save_storage_shrink(operation, ShrinkWorkspaceUpdate::None)
                        .await
                        .map_err(job_error)?;
                }
                for role in ["home", "temporary"] {
                    let selected = volume(operation, role);
                    if let Some(target) = selected.target_gib {
                        if selected.destination.is_none() {
                            let source = selected
                                .source
                                .as_ref()
                                .ok_or_else(|| job_error("source claim is missing"))?;
                            let destination = self
                                .coordinator
                                .create_shrink_destination(
                                    workspace, operation, role, source, target,
                                )
                                .await
                                .map_err(job_error)?;
                            volume_mut(operation, role).destination = Some(destination);
                        }
                    }
                }
                operation.phase = Phase::Copying;
                operation.updated_at = now;
                self.database
                    .save_storage_shrink(operation, ShrinkWorkspaceUpdate::None)
                    .await
                    .map_err(job_error)?;
            }
            Phase::Copying | Phase::Verifying => {
                let stage = if operation.phase == Phase::Copying {
                    ShrinkPodStage::Copy
                } else {
                    ShrinkPodStage::Verify
                };
                for role in ["home", "temporary"] {
                    if volume(operation, role).target_gib.is_none() {
                        continue;
                    }
                    let Some(usage) = self
                        .run_shrink_pod(workspace, operation, role, stage, now)
                        .await?
                    else {
                        return pending("offline copy/checksum verification is running");
                    };
                    if volume(operation, role).used_bytes != Some(usage.used_bytes)
                        || volume(operation, role).used_inodes != Some(usage.used_inodes)
                    {
                        return Err(job_error(
                            "source usage changed while the workspace was offline",
                        ));
                    }
                    if operation.phase == Phase::Verifying {
                        let destination = volume(operation, role)
                            .destination
                            .as_ref()
                            .ok_or_else(|| job_error("destination binding is missing"))?;
                        let identity = self
                            .coordinator
                            .shrink_volume_identity(workspace, destination)
                            .await
                            .map_err(job_error)?;
                        volume_mut(operation, role).destination_volume = Some(identity);
                        volume_mut(operation, role).verified = true;
                    }
                }
                operation.phase = if operation.phase == Phase::Copying {
                    Phase::Verifying
                } else {
                    Phase::Switching
                };
                operation.updated_at = now;
                self.database
                    .save_storage_shrink(operation, ShrinkWorkspaceUpdate::None)
                    .await
                    .map_err(job_error)?;
            }
            Phase::Switching => {
                if !self
                    .coordinator
                    .remove_shrink_helpers(workspace, operation)
                    .await
                    .map_err(job_error)?
                {
                    return pending(
                        "unmounting copy/verification Pods before the atomic binding exchange",
                    );
                }
                if !self
                    .coordinator
                    .stop_for_storage_shrink(workspace, operation)
                    .await
                    .map_err(job_error)?
                {
                    return pending("waiting for exclusive offline claims before switching");
                }
                operation.switched = true;
                operation.phase = Phase::Resuming;
                operation.updated_at = now;
                self.database
                    .save_storage_shrink(operation, ShrinkWorkspaceUpdate::Switch)
                    .await
                    .map_err(job_error)?;
            }
            Phase::Resuming => {
                if operation.resume {
                    if workspace.state == WorkspaceState::Stopped {
                        operation.updated_at = now;
                        self.database
                            .save_storage_shrink(operation, ShrinkWorkspaceUpdate::Resume)
                            .await
                            .map_err(job_error)?;
                        return pending("starting workspace with verified replacement volumes");
                    }
                    if workspace.state != WorkspaceState::Ready {
                        self.reconcile_workspace(workspace, job).await?;
                        return pending("waiting for workspace recovery readiness");
                    }
                } else {
                    self.reconcile_workspace(workspace, job).await?;
                }
                operation.phase = Phase::Completed;
                operation.reclaimable = true;
                if operation.request.reclaim_source {
                    operation.phase = Phase::Reclaiming;
                }
                operation.updated_at = now;
                self.database
                    .save_storage_shrink(operation, ShrinkWorkspaceUpdate::None)
                    .await
                    .map_err(job_error)?;
                if operation.phase == Phase::Reclaiming {
                    return pending(
                        "recovered workspace is ready; reclaiming sources under the original explicit consent",
                    );
                }
                return Ok(());
            }
            Phase::RollingBack => {
                if !self
                    .coordinator
                    .remove_shrink_helpers(workspace, operation)
                    .await
                    .map_err(job_error)?
                {
                    return pending("removing migration Pods before recovery");
                }
                if !self
                    .coordinator
                    .stop_for_storage_shrink(workspace, operation)
                    .await
                    .map_err(job_error)?
                {
                    return pending(
                        "waiting for offline recovery; source and destination claims are retained",
                    );
                }
                operation.phase = Phase::Failed;
                operation.updated_at = now;
                let update = if operation.switched {
                    ShrinkWorkspaceUpdate::Stopped
                } else {
                    ShrinkWorkspaceUpdate::Rollback
                };
                self.database
                    .save_storage_shrink(operation, update)
                    .await
                    .map_err(job_error)?;
                return Ok(());
            }
            Phase::Reclaiming => {
                if operation.resume && workspace.state != WorkspaceState::Ready {
                    return Err(job_error(
                        "workspace must be Ready before confirmed source reclamation",
                    ));
                }
                if !self
                    .coordinator
                    .reclaim_shrink_sources(workspace, operation)
                    .await
                    .map_err(job_error)?
                {
                    return pending("waiting for UID-verified source PVC reclamation");
                }
                operation.phase = Phase::Reclaimed;
                operation.sources_retained = false;
                operation.reclaimable = false;
                operation.updated_at = now;
                self.database
                    .save_storage_shrink(operation, ShrinkWorkspaceUpdate::None)
                    .await
                    .map_err(job_error)?;
                return Ok(());
            }
            Phase::Completed | Phase::Failed | Phase::Reclaimed => return Ok(()),
        }
        pending("storage migration advanced to its next durable phase")
    }

    async fn finish_preflight_failure(
        &self,
        workspace: &Workspace,
        operation: &mut StorageShrinkOperation,
        now: i64,
    ) -> Result<(), JobHandlerError> {
        if !self
            .coordinator
            .remove_shrink_helpers(workspace, operation)
            .await
            .map_err(job_error)?
        {
            return pending(
                "cleaning preflight helpers; the workspace remains in its original state",
            );
        }
        operation.phase = StorageShrinkPhase::Failed;
        operation.updated_at = now;
        self.database
            .save_storage_shrink(operation, ShrinkWorkspaceUpdate::None)
            .await
            .map_err(job_error)?;
        Ok(())
    }

    async fn run_shrink_pod(
        &self,
        workspace: &Workspace,
        operation: &mut StorageShrinkOperation,
        role: &str,
        stage: ShrinkPodStage,
        now: i64,
    ) -> Result<Option<crate::kubernetes::ShrinkUsage>, JobHandlerError> {
        let previous = operation.helper_pod_uids.clone();
        let result = self
            .coordinator
            .shrink_pod(workspace, operation, role, stage)
            .await;
        if previous != operation.helper_pod_uids {
            operation.updated_at = now;
            self.database
                .save_storage_shrink(operation, ShrinkWorkspaceUpdate::None)
                .await
                .map_err(job_error)?;
        }
        result.map_err(job_error)
    }

    pub(super) async fn storage_shrink_terminal_failure(
        &self,
        job: &ClaimedJob,
        now: i64,
    ) -> Result<(), StorageError> {
        let Some(workspace_id) = job.workspace_id else {
            return Ok(());
        };
        let Some(mut operation) = self.database.latest_storage_shrink(workspace_id).await? else {
            return Ok(());
        };
        if !operation.phase.active()
            || job.payload.get("operation_id").and_then(|id| id.as_str())
                != Some(operation.id.to_string().as_str())
        {
            return Ok(());
        }
        operation.error.get_or_insert_with(|| {
            "background migration exhausted retries; recovery is preserving all volumes".to_owned()
        });
        operation.updated_at = now;
        if operation.phase == StorageShrinkPhase::Queued {
            self.database
                .save_storage_shrink(&operation, ShrinkWorkspaceUpdate::None)
                .await?;
            self.database
                .enqueue_job(
                    NewJob {
                        kind: "workspace_storage_shrink".to_owned(),
                        workspace_id: Some(workspace_id),
                        payload: serde_json::json!({"operation_id": operation.id}),
                        available_at: now.saturating_add(60),
                    },
                    now,
                )
                .await?;
            return Ok(());
        }
        if operation.phase == StorageShrinkPhase::Reclaiming {
            operation.phase = StorageShrinkPhase::Completed;
            return self
                .database
                .save_storage_shrink(&operation, ShrinkWorkspaceUpdate::None)
                .await;
        }
        operation.phase = StorageShrinkPhase::RollingBack;
        self.database
            .save_storage_shrink(&operation, ShrinkWorkspaceUpdate::Stopping)
            .await?;
        self.database
            .enqueue_job(
                NewJob {
                    kind: "workspace_storage_shrink".to_owned(),
                    workspace_id: Some(workspace_id),
                    payload: serde_json::json!({"operation_id": operation.id}),
                    available_at: now.saturating_add(60),
                },
                now,
            )
            .await?;
        Ok(())
    }
}

fn volume<'operation>(
    operation: &'operation StorageShrinkOperation,
    role: &str,
) -> &'operation StorageShrinkVolume {
    if role == "home" {
        &operation.home
    } else {
        &operation.temporary
    }
}

fn volume_mut<'operation>(
    operation: &'operation mut StorageShrinkOperation,
    role: &str,
) -> &'operation mut StorageShrinkVolume {
    if role == "home" {
        &mut operation.home
    } else {
        &mut operation.temporary
    }
}

fn pending(message: &str) -> Result<(), JobHandlerError> {
    Err(JobHandlerError::Pending(message.to_owned()))
}
