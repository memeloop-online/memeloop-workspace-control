use crate::{
    kubernetes::storage_growth::GrowthError,
    storage::{ClaimedJob, StorageError, workspace_storage_growth::StorageGrowth},
    workspaces::WorkspaceState,
};

use super::{JobHandlerError, WorkspaceReconcileHandler, job_error, unix_timestamp};

impl WorkspaceReconcileHandler {
    pub(super) async fn handle_storage_growth(
        &self,
        job: &ClaimedJob,
    ) -> Result<(), JobHandlerError> {
        let id = job
            .workspace_id
            .ok_or_else(|| job_error("storage growth has no workspace"))?;
        let mut growth: StorageGrowth =
            serde_json::from_value(job.payload.clone()).map_err(job_error)?;
        if let Some(reason) = growth.rejection {
            return Err(JobHandlerError::Rejected(reason));
        }
        let pending = |error: StorageError| JobHandlerError::Pending(error.to_string());
        let workspace = match self.database.get_workspace(id).await {
            Ok(workspace) => workspace,
            Err(StorageError::WorkspaceNotFound) => return Ok(()),
            Err(error) => return Err(pending(error)),
        };
        if matches!(
            workspace.state,
            WorkspaceState::Deleting | WorkspaceState::Deleted
        ) {
            return Ok(());
        }
        if growth.volume.capacity(&workspace) == growth.requested_gib {
            return Ok(());
        }
        match self
            .coordinator
            .grow_workspace_claim(
                &workspace,
                &growth.claim,
                growth.previous_gib,
                growth.requested_gib,
            )
            .await
        {
            Ok(true) => self
                .database
                .apply_storage_growth(id, &growth, unix_timestamp()?)
                .await
                .map_err(pending),
            Ok(false) => Err(JobHandlerError::Pending(
                "waiting for the filesystem to grow".to_owned(),
            )),
            Err(GrowthError::Rejected) => {
                let reason = GrowthError::Rejected.to_string();
                self.database
                    .reject_storage_growth(id, job.id, &mut growth, &reason)
                    .await
                    .map_err(pending)?;
                Err(JobHandlerError::Rejected(reason))
            }
            Err(error) => Err(JobHandlerError::Pending(error.to_string())),
        }
    }
}
