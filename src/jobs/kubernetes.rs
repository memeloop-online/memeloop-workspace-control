use crate::{
    crypto::EnvelopeCipher,
    injections::{
        InjectionScope, InjectionSelection, filter_injection_refs, resolve_injections,
        select_injections,
    },
    jobs::{JobHandler, JobHandlerError},
    kubernetes::{DeleteProgress, KubernetesCoordinator, ResourceBuilder, WorkspaceResourceSpec},
    storage::{ClaimedJob, Database, InjectionScopeRef},
    workspaces::{Workspace, WorkspaceAction, WorkspaceState},
};

pub struct WorkspaceReconcileHandler {
    database: Database,
    cipher: EnvelopeCipher,
    builder: ResourceBuilder,
    coordinator: KubernetesCoordinator,
}

impl WorkspaceReconcileHandler {
    pub fn new(
        database: Database,
        cipher: EnvelopeCipher,
        builder: ResourceBuilder,
        coordinator: KubernetesCoordinator,
    ) -> Self {
        Self {
            database,
            cipher,
            builder,
            coordinator,
        }
    }

    async fn reconcile_workspace(&self, workspace: &Workspace) -> Result<(), JobHandlerError> {
        if workspace.state == WorkspaceState::Deleted {
            return Ok(());
        }
        if workspace.state == WorkspaceState::Deleting {
            return self.reconcile_deletion(workspace).await;
        }

        let resolved = self.resolve_workspace_injections(workspace).await?;
        let materialization = self
            .builder
            .materialize_injections(workspace.id, &workspace.short_id, &resolved)
            .map_err(job_error)?;
        let identity = self
            .database
            .ensure_workspace_ssh_identity(&self.cipher, workspace.id, unix_timestamp()?)
            .await
            .map_err(job_error)?;
        let ssh_identity = self
            .builder
            .materialize_ssh_identity(workspace.id, &workspace.short_id, &identity)
            .map_err(job_error)?;
        self.coordinator
            .reconcile_with_injections(&workspace_spec(workspace), materialization, ssh_identity)
            .await
            .map_err(job_error)?;

        match workspace.state {
            WorkspaceState::Provisioning
            | WorkspaceState::Starting
            | WorkspaceState::Restarting => {
                if !self
                    .coordinator
                    .has_observed_replicas(&workspace.short_id, 1)
                    .await
                    .map_err(job_error)?
                {
                    return Err(JobHandlerError(
                        "workspace StatefulSet is not ready yet".to_owned(),
                    ));
                }
                self.transition(workspace, WorkspaceAction::MarkReady)
                    .await?;
            }
            WorkspaceState::Stopping => {
                if !self
                    .coordinator
                    .has_observed_replicas(&workspace.short_id, 0)
                    .await
                    .map_err(job_error)?
                {
                    return Err(JobHandlerError(
                        "workspace StatefulSet has not scaled to zero yet".to_owned(),
                    ));
                }
                self.transition(workspace, WorkspaceAction::MarkStopped)
                    .await?;
            }
            WorkspaceState::Ready | WorkspaceState::Stopped | WorkspaceState::Failed => {}
            WorkspaceState::Deleting | WorkspaceState::Deleted => unreachable!(),
        }
        Ok(())
    }

    async fn reconcile_deletion(&self, workspace: &Workspace) -> Result<(), JobHandlerError> {
        match self
            .coordinator
            .delete_or_confirm(workspace.id, &workspace.short_id)
            .await
            .map_err(job_error)?
        {
            DeleteProgress::Gone => {
                self.transition(workspace, WorkspaceAction::MarkDeleted)
                    .await?;
                Ok(())
            }
            DeleteProgress::DeletionRequested | DeleteProgress::Terminating => Err(
                JobHandlerError("workspace namespace deletion is still in progress".to_owned()),
            ),
        }
    }

    async fn resolve_workspace_injections(
        &self,
        workspace: &Workspace,
    ) -> Result<Vec<crate::injections::ResolvedInjection>, JobHandlerError> {
        let organization = self
            .database
            .load_injections(
                &self.cipher,
                InjectionScopeRef {
                    scope: InjectionScope::Organization,
                    scope_id: workspace.organization_id,
                },
            )
            .await
            .map_err(job_error)?;
        let user = self
            .database
            .load_injections(
                &self.cipher,
                InjectionScopeRef {
                    scope: InjectionScope::User,
                    scope_id: workspace.owner_id,
                },
            )
            .await
            .map_err(job_error)?;
        let workspace_items = self
            .database
            .load_injections(
                &self.cipher,
                InjectionScopeRef {
                    scope: InjectionScope::Workspace,
                    scope_id: workspace.id,
                },
            )
            .await
            .map_err(job_error)?;
        let selection = InjectionSelection {
            workspace_id: Some(workspace.id),
            organization_id: workspace.organization_id,
            owner_id: workspace.owner_id,
            template_id: workspace.template_id,
            image: &workspace.image,
            access_mode: workspace.access_mode,
        };
        let refs = self
            .database
            .workspace_injection_refs(workspace.id)
            .await
            .map_err(job_error)?;
        let organization = filter_injection_refs(
            &select_injections(&organization, selection),
            refs.organization.as_deref(),
            true,
        );
        let user = filter_injection_refs(
            &select_injections(&user, selection),
            refs.user.as_deref(),
            false,
        );
        let workspace_items = select_injections(&workspace_items, selection);
        let mut resolved =
            resolve_injections(&organization, &user, &workspace_items).map_err(job_error)?;
        normalize_resolved_ssh_targets(&mut resolved, "cascade");

        for candidate in self
            .database
            .ssh_access_candidates(workspace.organization_id)
            .await
            .map_err(job_error)?
        {
            if candidate.user_id == workspace.owner_id {
                continue;
            }
            let items = self
                .database
                .load_injections(
                    &self.cipher,
                    InjectionScopeRef {
                        scope: InjectionScope::User,
                        scope_id: candidate.user_id,
                    },
                )
                .await
                .map_err(job_error)?;
            for (index, mut item) in select_injections(&items, selection)
                .into_iter()
                .filter(|item| item.kind == crate::injections::InjectionKind::SshPublicKey)
                .enumerate()
            {
                item.key = format!("mwc-access-{}-{index}", candidate.user_id);
                item.target = format!("/workspace/.mwc/access-{}-{index}.pub", candidate.user_id);
                item.locked = false;
                item.sensitive = false;
                resolved.push(crate::injections::ResolvedInjection {
                    source: InjectionScope::User,
                    item,
                });
            }
        }
        Ok(resolved)
    }

    async fn transition(
        &self,
        workspace: &Workspace,
        action: WorkspaceAction,
    ) -> Result<(), JobHandlerError> {
        self.database
            .transition_workspace(workspace.id, action, workspace.owner_id, unix_timestamp()?)
            .await
            .map_err(job_error)?;
        Ok(())
    }
}

fn normalize_resolved_ssh_targets(
    resolved: &mut [crate::injections::ResolvedInjection],
    prefix: &str,
) {
    for (index, item) in resolved
        .iter_mut()
        .filter(|item| item.item.kind == crate::injections::InjectionKind::SshPublicKey)
        .enumerate()
    {
        item.item.target = format!("/workspace/.mwc/{prefix}-{index}.pub");
        item.item.sensitive = false;
    }
}

impl JobHandler for WorkspaceReconcileHandler {
    async fn handle(&self, job: &ClaimedJob) -> Result<(), JobHandlerError> {
        if job.kind != "reconcile_workspace" {
            return Err(JobHandlerError(format!(
                "unsupported background job kind {}",
                job.kind
            )));
        }
        let workspace_id = job
            .workspace_id
            .ok_or_else(|| JobHandlerError("workspace job has no workspace id".to_owned()))?;
        let workspace = self
            .database
            .get_workspace(workspace_id)
            .await
            .map_err(job_error)?;
        self.reconcile_workspace(&workspace).await
    }
}

fn workspace_spec(workspace: &Workspace) -> WorkspaceResourceSpec {
    WorkspaceResourceSpec {
        id: workspace.id,
        short_id: workspace.short_id.clone(),
        image: workspace.image.clone(),
        resources: workspace.resources,
        access_mode: workspace.access_mode,
        state: workspace.state,
        generation: workspace.generation,
    }
}

fn job_error(error: impl std::fmt::Display) -> JobHandlerError {
    JobHandlerError(error.to_string())
}

fn unix_timestamp() -> Result<i64, JobHandlerError> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(job_error)?
        .as_secs();
    i64::try_from(seconds).map_err(job_error)
}
