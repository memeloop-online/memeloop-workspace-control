use crate::{
    injections::{
        InjectionItem, InjectionScope, InjectionSelection, filter_injection_refs,
        resolve_injections, select_injections, validate_injection_item,
    },
    storage::{CreateWorkspace, InjectionScopeRef},
    workspaces::{Workspace, WorkspaceState},
};

use super::{ApiError, AppState};

pub(super) async fn validate_inline_injections(
    state: &AppState,
    command: &CreateWorkspace,
    inline: &[InjectionItem],
) -> Result<(), ApiError> {
    for item in inline {
        validate_injection_item(item)?;
    }
    if inline.iter().any(|item| item.locked) {
        return Err(ApiError::BadRequest(
            "workspace inline injections cannot be locked",
        ));
    }
    let Some(cipher) = state.cipher.as_ref() else {
        return if inline.is_empty()
            && command.organization_injection_refs.is_none()
            && command.user_injection_refs.is_none()
        {
            Ok(())
        } else {
            Err(ApiError::EncryptionUnavailable)
        };
    };
    let organization = state
        .database
        .load_injections(
            cipher,
            InjectionScopeRef {
                scope: InjectionScope::Organization,
                scope_id: command.organization_id,
            },
        )
        .await?;
    let user = state
        .database
        .load_injections(
            cipher,
            InjectionScopeRef {
                scope: InjectionScope::User,
                scope_id: command.owner_id,
            },
        )
        .await?;
    let template = state
        .database
        .get_workspace_template(command.template_id)
        .await?;
    if !template.enabled
        || template
            .organization_id
            .is_some_and(|id| id != command.organization_id)
    {
        return Err(ApiError::from(
            crate::storage::StorageError::TemplateNotFound,
        ));
    }
    let selection = InjectionSelection {
        workspace_id: None,
        organization_id: command.organization_id,
        owner_id: command.owner_id,
        template_id: Some(command.template_id),
        image: &template.template.image,
        access_mode: template.template.access_mode,
    };
    let organization = select_injections(&organization, selection);
    let user = select_injections(&user, selection);
    validate_refs(
        command.organization_injection_refs.as_deref(),
        &organization,
    )?;
    validate_refs(command.user_injection_refs.as_deref(), &user)?;
    resolve_injections(
        &filter_injection_refs(
            &organization,
            command.organization_injection_refs.as_deref(),
            true,
        ),
        &filter_injection_refs(&user, command.user_injection_refs.as_deref(), false),
        &select_injections(inline, selection),
    )?;
    Ok(())
}

pub(super) fn validate_refs(
    refs: Option<&[String]>,
    available: &[InjectionItem],
) -> Result<(), ApiError> {
    let Some(refs) = refs else {
        return Ok(());
    };
    let mut sorted = refs.to_vec();
    sorted.sort();
    if sorted.windows(2).any(|pair| pair[0] == pair[1])
        || refs
            .iter()
            .any(|key| !available.iter().any(|item| item.key == *key))
    {
        return Err(ApiError::BadRequest(
            "workspace injection references must be unique keys available after selector filtering",
        ));
    }
    Ok(())
}

pub(super) async fn wait_until_ready(
    state: &AppState,
    mut workspace: Workspace,
    timeout_seconds: u64,
) -> Result<Workspace, ApiError> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_seconds);
    while tokio::time::Instant::now() < deadline {
        if matches!(
            workspace.state,
            WorkspaceState::Ready | WorkspaceState::Failed | WorkspaceState::Deleting
        ) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        workspace = state.database.get_workspace(workspace.id).await?;
    }
    Ok(workspace)
}
