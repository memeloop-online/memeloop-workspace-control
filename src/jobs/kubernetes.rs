use uuid::Uuid;

use crate::{
    crypto::EnvelopeCipher,
    injections::{
        InjectionScope, InjectionSelection, filter_injection_refs, resolve_injections,
        select_injections,
    },
    jobs::{JobHandler, JobHandlerError},
    kubernetes::{
        DeleteProgress, InjectionMaterialization, KubernetesCoordinator, ReconcileError,
        ResourceBuilder, WorkspaceObservations,
    },
    storage::{ClaimedJob, Database, InjectionScopeRef, StorageError},
    workspaces::{Workspace, WorkspaceObservation, WorkspaceState},
};

#[path = "storage_shrink.rs"]
mod storage_shrink;

pub struct WorkspaceReconcileHandler {
    database: Database,
    cipher: EnvelopeCipher,
    builder: ResourceBuilder,
    coordinator: KubernetesCoordinator,
    observations: Option<WorkspaceObservations>,
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
            observations: None,
        }
    }

    pub fn with_observations(mut self, observations: WorkspaceObservations) -> Self {
        self.observations = Some(observations);
        self
    }

    async fn reconcile_workspace(
        &self,
        workspace: &Workspace,
        job: &ClaimedJob,
    ) -> Result<(), JobHandlerError> {
        if job
            .payload
            .get("runtime_observation")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        {
            // Watch hints never authorize resource writes or replay an old intent.
            if job
                .payload
                .get("generation")
                .and_then(serde_json::Value::as_u64)
                != Some(workspace.generation)
                || !matches!(
                    workspace.state,
                    WorkspaceState::Ready | WorkspaceState::Starting
                )
            {
                return Ok(());
            }
            let now = unix_timestamp()?;
            return self.observe_workspace(workspace).await.map_err(|error| {
                let error = match error {
                    JobHandlerError::Failed(message) => JobHandlerError::Pending(message),
                    error => error,
                };
                checkpoint_error(error, job, workspace, Some(now))
            });
        }

        if workspace.state == WorkspaceState::Deleted {
            return Ok(());
        }
        if workspace.state == WorkspaceState::Deleting {
            return self.reconcile_deletion(workspace).await;
        }

        if startup_checkpoint_matches(workspace, job, unix_timestamp()?) {
            return self
                .observe_workspace(workspace)
                .await
                .map_err(|error| checkpoint_error(error, job, workspace, None));
        }

        let materialization = self.materialize_workspace_injections(workspace).await?;
        let identity = self
            .database
            .ensure_workspace_ssh_identity(&self.cipher, workspace.id, unix_timestamp()?)
            .await
            .map_err(job_error)?;
        let ssh_identity = self
            .builder
            .materialize_ssh_identity(
                workspace.id,
                &workspace.short_id,
                &workspace.runtime,
                &identity,
            )
            .map_err(job_error)?;
        let placement = self
            .database
            .resolve_workspace_placement(&workspace.node_pool)
            .await
            .map_err(job_error)?;
        self.coordinator
            .reconcile_with_injections(workspace, &placement, materialization, ssh_identity)
            .await
            .map_err(reconcile_error)?;
        self.database
            .ensure_desktop_port_mapping(workspace, unix_timestamp()?)
            .await
            .map_err(job_error)?;
        let port_mappings = if matches!(
            workspace.state,
            WorkspaceState::Provisioning
                | WorkspaceState::Starting
                | WorkspaceState::Restarting
                | WorkspaceState::Ready
        ) {
            self.database
                .list_port_mappings(workspace.id)
                .await
                .map_err(job_error)?
        } else {
            Vec::new()
        };
        self.coordinator
            .reconcile_port_mappings(workspace, &port_mappings)
            .await
            .map_err(job_error)?;

        let reconciled_at = unix_timestamp()?;
        self.observe_workspace(workspace)
            .await
            .map_err(|error| checkpoint_error(error, job, workspace, Some(reconciled_at)))
    }

    async fn observe_workspace(&self, workspace: &Workspace) -> Result<(), JobHandlerError> {
        match workspace.state {
            WorkspaceState::Provisioning
            | WorkspaceState::Starting
            | WorkspaceState::Restarting => {
                if !self
                    .coordinator
                    .has_observed_replicas(workspace, 1)
                    .await
                    .map_err(job_error)?
                {
                    return Err(JobHandlerError::Pending(
                        "workspace StatefulSet is not ready yet".to_owned(),
                    ));
                }
                self.record_observation(workspace, WorkspaceObservation::Ready)
                    .await?;
            }
            WorkspaceState::Ready => {
                if !self
                    .coordinator
                    .has_observed_replicas(workspace, 1)
                    .await
                    .map_err(|error| JobHandlerError::Pending(error.to_string()))?
                {
                    self.record_observation(workspace, WorkspaceObservation::NotReady)
                        .await?;
                    return Err(JobHandlerError::Pending(
                        "workspace Pod is recovering readiness".to_owned(),
                    ));
                }
            }
            WorkspaceState::Stopping => {
                if !self
                    .coordinator
                    .has_observed_replicas(workspace, 0)
                    .await
                    .map_err(job_error)?
                {
                    return Err(JobHandlerError::Pending(
                        "workspace StatefulSet has not scaled to zero yet".to_owned(),
                    ));
                }
                if !Box::pin(self.coordinator.delete_stopped_scratch(workspace))
                    .await
                    .map_err(job_error)?
                {
                    return Err(JobHandlerError::Pending(
                        "waiting for temporary scratch PVC removal".to_owned(),
                    ));
                }
                self.record_observation(workspace, WorkspaceObservation::Stopped)
                    .await?;
            }
            WorkspaceState::Stopped | WorkspaceState::Failed => {}
            WorkspaceState::Deleting | WorkspaceState::Deleted => unreachable!(),
        }
        Ok(())
    }

    async fn reconcile_deletion(&self, workspace: &Workspace) -> Result<(), JobHandlerError> {
        let mut operations = self
            .database
            .list_storage_shrinks(workspace.id)
            .await
            .map_err(job_error)?;
        for operation in &mut operations {
            self.coordinator
                .capture_shrink_cleanup_identities(workspace, operation)
                .await
                .map_err(job_error)?;
            self.database
                .save_storage_shrink_cleanup_identity(operation)
                .await
                .map_err(job_error)?;
        }
        match self
            .coordinator
            .delete_or_confirm(workspace)
            .await
            .map_err(job_error)?
        {
            DeleteProgress::Gone => {
                if !self
                    .coordinator
                    .delete_workspace_shrink_storage(workspace, &operations)
                    .await
                    .map_err(job_error)?
                {
                    return Err(JobHandlerError::Pending(
                        "waiting for storage-shrink bindings and backups to be deleted".to_owned(),
                    ));
                }
                self.record_observation(workspace, WorkspaceObservation::Deleted)
                    .await?;
                Ok(())
            }
            DeleteProgress::DeletionRequested | DeleteProgress::Terminating => {
                Err(JobHandlerError::Pending(
                    "workspace namespace deletion is still in progress".to_owned(),
                ))
            }
        }
    }

    async fn materialize_workspace_injections(
        &self,
        workspace: &Workspace,
    ) -> Result<InjectionMaterialization, JobHandlerError> {
        let resolved = self.resolve_workspace_injections(workspace).await?;
        self.builder
            .materialize_injections(
                workspace.id,
                &workspace.short_id,
                &workspace.runtime,
                &resolved,
            )
            .map_err(job_error)
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
            image: &workspace.template.image,
            access_mode: workspace.template.access_mode,
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
        normalize_resolved_ssh_items(&mut resolved, &workspace.template);
        self.append_shared_ssh_keys(workspace, selection, &mut resolved)
            .await?;
        Ok(resolved)
    }

    async fn append_shared_ssh_keys(
        &self,
        workspace: &Workspace,
        selection: InjectionSelection<'_>,
        resolved: &mut Vec<crate::injections::ResolvedInjection>,
    ) -> Result<(), JobHandlerError> {
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
                normalize_ssh_target(
                    &mut item,
                    &workspace.template,
                    &format!("access-{}-{index}", candidate.user_id),
                );
                item.locked = false;
                item.sensitive = false;
                resolved.push(crate::injections::ResolvedInjection {
                    source: InjectionScope::User,
                    item,
                });
            }
        }
        Ok(())
    }

    async fn record_observation(
        &self,
        workspace: &Workspace,
        observation: WorkspaceObservation,
    ) -> Result<(), JobHandlerError> {
        let started = std::time::Instant::now();
        let recorded = self
            .database
            .record_workspace_observation(
                workspace.id,
                workspace.generation,
                observation,
                workspace.owner_id,
                unix_timestamp()?,
            )
            .await
            .map_err(job_error)?;
        if let Some(recorded) = recorded {
            // Audit timestamps are supplied before acquiring the transaction.
            // This log is emitted after commit, so diagnostics can separate them.
            tracing::info!(
                workspace_id = %recorded.id,
                workspace = %recorded.name,
                generation = recorded.generation,
                state = ?recorded.state,
                elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0,
                "workspace observation committed"
            );
        }
        Ok(())
    }
}

fn normalize_resolved_ssh_items(
    resolved: &mut [crate::injections::ResolvedInjection],
    template: &crate::templates::WorkspaceTemplateSpec,
) {
    let login_user = template.workspace_user.as_str();
    for item in resolved
        .iter_mut()
        .filter(|item| item.item.kind == crate::injections::InjectionKind::SshPublicKey)
    {
        let name = format!("injected-{}", item.item.key);
        normalize_ssh_target(&mut item.item, template, &name);
        item.item.sensitive = false;
        item.item.file_mode = Some(0o600);
        item.item.owner = Some(login_user.to_owned());
        item.item.group = Some(login_user.to_owned());
    }
}

fn normalize_ssh_target(
    item: &mut crate::injections::InjectionItem,
    template: &crate::templates::WorkspaceTemplateSpec,
    name: &str,
) {
    let login_user = template.workspace_user.as_str();
    item.target = format!("{}/.mwc/{name}.pub", template.workspace_home);
    item.file_mode = Some(0o600);
    item.owner = Some(login_user.to_owned());
    item.group = Some(login_user.to_owned());
}

fn startup_checkpoint_matches(workspace: &Workspace, job: &ClaimedJob, now: i64) -> bool {
    matches!(
        workspace.state,
        WorkspaceState::Provisioning
            | WorkspaceState::Starting
            | WorkspaceState::Restarting
            | WorkspaceState::Stopping
    ) && job
        .payload
        .get("reconciled_generation")
        .and_then(serde_json::Value::as_u64)
        == Some(workspace.generation)
        && job
            .payload
            .get("reconciled_at")
            .and_then(serde_json::Value::as_i64)
            .is_some_and(|at| (0..60).contains(&now.saturating_sub(at)))
}

fn checkpoint_error(
    error: JobHandlerError,
    job: &ClaimedJob,
    workspace: &Workspace,
    reconciled_at: Option<i64>,
) -> JobHandlerError {
    let JobHandlerError::Pending(message) = error else {
        return error;
    };
    let mut payload = job.payload.clone();
    if let Some(object) = payload.as_object_mut() {
        object.remove("runtime_wait");
    }
    if let Some(at) = reconciled_at {
        payload["reconciled_generation"] = workspace.generation.into();
        payload["reconciled_at"] = at.into();
    }
    JobHandlerError::PendingWithCheckpoint { message, payload }
}

impl JobHandler for WorkspaceReconcileHandler {
    async fn handle(&self, job: &ClaimedJob) -> Result<(), JobHandlerError> {
        if job.kind == "workspace_storage_shrink" {
            return self.handle_storage_shrink(job).await;
        }
        if job.kind != "reconcile_workspace" {
            return Err(JobHandlerError::Failed(format!(
                "unsupported background job kind {}",
                job.kind
            )));
        }
        let workspace_id = job.workspace_id.ok_or_else(|| {
            JobHandlerError::Failed("workspace job has no workspace id".to_owned())
        })?;
        let workspace = match self.database.get_workspace(workspace_id).await {
            Ok(workspace) => workspace,
            Err(StorageError::WorkspaceNotFound) => {
                tracing::info!(%workspace_id, %job.id, "discarding obsolete workspace job");
                return Ok(());
            }
            Err(error) => return Err(job_error(error)),
        };
        if self
            .database
            .storage_shrink_active(workspace_id)
            .await
            .map_err(job_error)?
        {
            return Ok(());
        }
        if job
            .payload
            .get("reason")
            .and_then(serde_json::Value::as_str)
            == Some("injection_changed")
        {
            if matches!(
                workspace.state,
                WorkspaceState::Deleting | WorkspaceState::Deleted
            ) {
                return Ok(());
            }
            let materialization = self.materialize_workspace_injections(&workspace).await?;
            return self
                .coordinator
                .reconcile_injections(&workspace, &materialization)
                .await
                .map_err(reconcile_error);
        }
        self.reconcile_workspace(&workspace, job).await
    }

    async fn on_checkpoint_deferred(&self, job_id: Uuid, now: i64) -> Result<(), StorageError> {
        if let Some(observations) = &self.observations {
            observations.register(job_id, now).await?;
        }
        Ok(())
    }

    async fn on_terminal_failure(&self, job: &ClaimedJob, now: i64) -> Result<(), StorageError> {
        if job.kind == "workspace_storage_shrink" {
            return self.storage_shrink_terminal_failure(job, now).await;
        }
        if job.kind != "reconcile_workspace"
            || job
                .payload
                .get("reason")
                .and_then(serde_json::Value::as_str)
                == Some("injection_changed")
            || job
                .payload
                .get("runtime_observation")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
        {
            return Ok(());
        }
        let Some(workspace_id) = job.workspace_id else {
            return Ok(());
        };
        let Some(expected_generation) = job
            .payload
            .get("generation")
            .and_then(|value| value.as_u64())
        else {
            tracing::debug!(job_id = %job.id, "terminal reconcile job has no generation guard");
            return Ok(());
        };
        if !self
            .database
            .mark_workspace_failed_if_generation(workspace_id, expected_generation, now)
            .await?
        {
            tracing::debug!(
                job_id = %job.id,
                %workspace_id,
                expected_generation,
                "ignoring terminal failure for a stale workspace reconcile generation"
            );
        }
        Ok(())
    }
}

fn job_error(error: impl std::fmt::Display) -> JobHandlerError {
    JobHandlerError::Failed(error.to_string())
}

fn reconcile_error(error: ReconcileError) -> JobHandlerError {
    match error {
        ReconcileError::StatefulSetRecreationPending
        | ReconcileError::ScratchVolumeRecreationPending
        | ReconcileError::UnpersistedHomeVolumeBinding { .. } => {
            JobHandlerError::Pending(error.to_string())
        }
        error => job_error(error),
    }
}

fn unix_timestamp() -> Result<i64, JobHandlerError> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(job_error)?
        .as_secs();
    i64::try_from(seconds).map_err(job_error)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::{
        injections::{
            InjectionItem, InjectionKind, InjectionScope, InjectionValue, ResolvedInjection,
        },
        quota::Resources,
        templates::WorkspaceTemplateSpec,
        workspaces::AccessMode,
    };

    use super::normalize_resolved_ssh_items;

    #[test]
    fn ssh_materialization_uses_the_runtime_home_and_ownership() {
        let mut resolved = vec![ResolvedInjection {
            source: InjectionScope::User,
            item: InjectionItem {
                key: "personal-key".to_owned(),
                kind: InjectionKind::SshPublicKey,
                target: "/wrong/location".to_owned(),
                value: InjectionValue::Utf8("ssh-ed25519 test".to_owned()),
                sensitive: true,
                locked: false,
                version: 1,
                file_mode: None,
                owner: None,
                group: None,
                template_selector: None,
                labels: BTreeMap::new(),
            },
        }];

        let mut template = WorkspaceTemplateSpec::standard(
            "registry.example/node:latest",
            AccessMode::Internal,
            Resources {
                cpu_millis: 6_000,
                memory_mib: 4_096,
                gpu_count: 0,
                disk_gib: 60,
            },
        );
        template.workspace_user = "node-dev".to_owned();
        template.workspace_home = "/home/node-dev".to_owned();
        normalize_resolved_ssh_items(&mut resolved, &template);

        let item = &resolved[0].item;
        assert_eq!(item.target, "/home/node-dev/.mwc/injected-personal-key.pub");
        assert_eq!(item.file_mode, Some(0o600));
        assert_eq!(item.owner.as_deref(), Some("node-dev"));
        assert_eq!(item.group.as_deref(), Some("node-dev"));
        assert!(!item.sensitive);
    }
}
