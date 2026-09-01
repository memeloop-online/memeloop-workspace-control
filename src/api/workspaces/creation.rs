use crate::{
    auth::Permission,
    plugins::{WorkspaceCreateContext, WorkspaceCreatePlan},
    storage::{CreateWorkspace, WorkspaceTemplate},
    workspaces::Workspace,
};

use super::super::workspace_creation::validate_inline_injections;
use super::{ApiError, AppState, CreateWorkspaceRequest};

pub(super) async fn create_admitted_workspace(
    state: &AppState,
    actor: &crate::storage::Principal,
    request: &CreateWorkspaceRequest,
    template: &WorkspaceTemplate,
    now: i64,
) -> Result<Workspace, ApiError> {
    let command = &request.workspace;
    let mut final_template = template.template.clone();
    if let Some(resources) = command.resources {
        final_template.resources = resources;
        final_template
            .validate()
            .map_err(|_| crate::storage::StorageError::InvalidWorkspace)?;
    }
    validate_inline_injections(
        state,
        command,
        template,
        &request.inline_workspace_injections,
    )
    .await?;
    state
        .plugins
        .admit_workspace_create(
            WorkspaceCreateContext {
                installation_id: state.config.installation_id.to_string(),
                actor_user_id: actor.user_id,
                organization_id: command.organization_id,
                owner_id: command.owner_id,
                template_id: command.template_id,
            },
            WorkspaceCreatePlan::from_template(&command.name, &final_template),
        )
        .await?;
    let inline = if request.inline_workspace_injections.is_empty() {
        None
    } else {
        Some((
            state
                .cipher
                .as_ref()
                .ok_or(ApiError::EncryptionUnavailable)?,
            request.inline_workspace_injections.as_slice(),
        ))
    };
    Ok(state
        .database
        .create_workspace_with_admitted_template(
            command.clone(),
            inline,
            &template.yaml,
            actor.may_manage_system(),
            actor.user_id,
            now,
        )
        .await?)
}

pub(super) async fn authorize_creation(
    state: &AppState,
    actor: &crate::storage::Principal,
    command: &CreateWorkspace,
) -> Result<WorkspaceTemplate, ApiError> {
    if !actor.allows(Permission::CreateWorkspace, command.organization_id) {
        return Err(ApiError::Forbidden);
    }
    if command.owner_id != actor.user_id
        && !actor.allows(Permission::ManageMembers, command.organization_id)
    {
        return Err(ApiError::Forbidden);
    }
    let template = state
        .database
        .get_workspace_template(command.template_id)
        .await?;
    if !template.enabled
        || template
            .organization_id
            .is_some_and(|id| id != command.organization_id)
    {
        return Err(crate::storage::StorageError::TemplateNotFound.into());
    }
    if template.template.cluster_access && !actor.may_manage_system() {
        return Err(ApiError::Forbidden);
    }
    Ok(template)
}
