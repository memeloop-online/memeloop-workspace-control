use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    auth::Permission,
    injections::{
        InjectionItem, InjectionScope, InjectionSelection, ResolvedInjectionSummary,
        filter_injection_refs, resolve_injections, select_injections, validate_injection_item,
    },
    storage::{IdempotencyDecision, InjectionScopeRef, Principal, StoredInjectionSummary},
};

#[path = "injections/delete.rs"]
mod deletion;

pub(super) use deletion::{__path_delete, delete};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(super) struct PreviewRequest {
    pub organization_id: Option<Uuid>,
    pub user_id: Uuid,
    pub workspace_id: Option<Uuid>,
    #[serde(default)]
    pub organization_injection_refs: Option<Vec<String>>,
    #[serde(default)]
    pub user_injection_refs: Option<Vec<String>>,
    #[serde(default)]
    pub inline_workspace_injections: Vec<InjectionItem>,
}

use super::{
    ApiError, AppState,
    auth::principal,
    idempotency::{
        IDEMPOTENCY_TTL_SECONDS, hash, idempotency_key, json_response, replay_response,
        unix_timestamp,
    },
};

#[utoipa::path(
    get,
    path = "/api/v1/injections/{scope}/{scope_id}",
    params(
        ("scope" = String, Path, description = "organization, user, or workspace"),
        ("scope_id" = Uuid, Path)
    ),
    responses(
        (status = 200, description = "Write-only injection metadata", body = [StoredInjectionSummary]),
        (status = 401, body = super::ErrorEnvelope),
        (status = 403, body = super::ErrorEnvelope)
    )
)]
pub(super) async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((scope, scope_id)): Path<(String, Uuid)>,
) -> Result<Json<Vec<StoredInjectionSummary>>, ApiError> {
    let actor = principal(&state, &headers).await?;
    let scope_ref = parse_scope(&scope, scope_id)?;
    authorize(&state, &actor, scope_ref, false, false).await?;
    Ok(Json(
        state.database.list_injection_summaries(scope_ref).await?,
    ))
}

#[utoipa::path(
    put,
    path = "/api/v1/injections/{scope}/{scope_id}/{key}",
    params(
        ("scope" = String, Path),
        ("scope_id" = Uuid, Path),
        ("key" = String, Path),
        ("Idempotency-Key" = String, Header)
    ),
    request_body = InjectionItem,
    responses(
        (status = 200, description = "Injection replaced; value is never returned", body = StoredInjectionSummary),
        (status = 401, body = super::ErrorEnvelope),
        (status = 403, body = super::ErrorEnvelope),
        (status = 409, body = super::ErrorEnvelope),
        (status = 503, body = super::ErrorEnvelope)
    )
)]
pub(super) async fn replace(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((scope, scope_id, key)): Path<(String, Uuid, String)>,
    Json(mut item): Json<InjectionItem>,
) -> Result<Response, ApiError> {
    if item.key != key {
        return Err(ApiError::BadRequest(
            "path key must exactly match the injection body key",
        ));
    }
    validate_injection_item(&item)?;
    item.version = 0;
    let actor = principal(&state, &headers).await?;
    let scope_ref = parse_scope(&scope, scope_id)?;
    authorize(&state, &actor, scope_ref, true, item.locked).await?;
    let cipher = state
        .cipher
        .as_ref()
        .ok_or(ApiError::EncryptionUnavailable)?;
    let idempotency_key = idempotency_key(&headers)?;
    let request_hash = hash(&item)?;
    let idempotency_scope = format!(
        "{}:replace-injection:{}:{}:{}",
        actor.user_id,
        scope_ref.scope.as_str(),
        scope_ref.scope_id,
        key
    );
    let now = unix_timestamp()?;
    match state
        .database
        .begin_idempotency(
            &idempotency_scope,
            idempotency_key,
            &request_hash,
            now,
            now + IDEMPOTENCY_TTL_SECONDS,
        )
        .await?
    {
        IdempotencyDecision::Replay(replay) => return replay_response(replay),
        IdempotencyDecision::Conflict => return Err(ApiError::IdempotencyConflict),
        IdempotencyDecision::InProgress => return Err(ApiError::IdempotencyInProgress),
        IdempotencyDecision::Reserved => {}
    }
    let summary = match state
        .database
        .replace_injection(cipher, scope_ref, item, actor.user_id, now)
        .await
    {
        Ok(summary) => summary,
        Err(error) => {
            state
                .database
                .abandon_idempotency(&idempotency_scope, idempotency_key, &request_hash)
                .await?;
            return Err(error.into());
        }
    };
    state
        .database
        .enqueue_injection_reconciles(scope_ref, now)
        .await?;
    let response_json = serde_json::to_string(&summary)
        .map_err(|_| ApiError::BadRequest("response serialization failed"))?;
    state
        .database
        .finish_idempotency(
            &idempotency_scope,
            idempotency_key,
            &request_hash,
            StatusCode::OK.as_u16(),
            &response_json,
        )
        .await?;
    json_response(StatusCode::OK, response_json)
}

#[utoipa::path(
    post,
    path = "/api/v1/injections/preview",
    request_body = PreviewRequest,
    responses(
        (status = 200, description = "Resolved source metadata without values", body = [ResolvedInjectionSummary]),
        (status = 400, body = super::ErrorEnvelope),
        (status = 401, body = super::ErrorEnvelope),
        (status = 403, body = super::ErrorEnvelope),
        (status = 409, body = super::ErrorEnvelope)
    )
)]
pub(super) async fn preview(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<PreviewRequest>,
) -> Result<Json<Vec<ResolvedInjectionSummary>>, ApiError> {
    for item in &request.inline_workspace_injections {
        validate_injection_item(item)?;
    }
    let actor = principal(&state, &headers).await?;
    let cipher = state
        .cipher
        .as_ref()
        .ok_or(ApiError::EncryptionUnavailable)?;

    let target = preview_target(&state, &actor, &request).await?;

    let organization_id = target
        .as_ref()
        .map(|workspace| workspace.organization_id)
        .or(request.organization_id);
    let user_id = target
        .as_ref()
        .map_or(request.user_id, |workspace| workspace.owner_id);

    let organization = if let Some(organization_id) = organization_id {
        let scope = InjectionScopeRef {
            scope: InjectionScope::Organization,
            scope_id: organization_id,
        };
        if target.is_none() {
            authorize(&state, &actor, scope, false, false).await?;
        }
        state.database.load_injections(cipher, scope).await?
    } else {
        Vec::new()
    };
    let user = state
        .database
        .load_injections(
            cipher,
            InjectionScopeRef {
                scope: InjectionScope::User,
                scope_id: user_id,
            },
        )
        .await?;
    let mut workspace = if let Some(workspace_id) = request.workspace_id {
        let scope = InjectionScopeRef {
            scope: InjectionScope::Workspace,
            scope_id: workspace_id,
        };
        state.database.load_injections(cipher, scope).await?
    } else {
        Vec::new()
    };
    workspace.retain(|stored| {
        !request
            .inline_workspace_injections
            .iter()
            .any(|inline| inline.key == stored.key)
    });
    workspace.extend(request.inline_workspace_injections);

    let (organization, user, workspace) = if let Some(target) = target.as_ref() {
        let selection = InjectionSelection {
            workspace_id: Some(target.id),
            organization_id: target.organization_id,
            owner_id: target.owner_id,
            template_id: target.template_id,
            image: &target.template.image,
            access_mode: target.template.access_mode,
        };
        let refs = state.database.workspace_injection_refs(target.id).await?;
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
        (organization, user, select_injections(&workspace, selection))
    } else {
        super::workspace_creation::validate_refs(
            request.organization_injection_refs.as_deref(),
            &organization,
        )?;
        super::workspace_creation::validate_refs(request.user_injection_refs.as_deref(), &user)?;
        (
            filter_injection_refs(
                &organization,
                request.organization_injection_refs.as_deref(),
                true,
            ),
            filter_injection_refs(&user, request.user_injection_refs.as_deref(), false),
            workspace,
        )
    };
    let resolved = resolve_injections(&organization, &user, &workspace)?;
    Ok(Json(
        resolved.into_iter().map(|item| item.summary()).collect(),
    ))
}

async fn preview_target(
    state: &AppState,
    actor: &Principal,
    request: &PreviewRequest,
) -> Result<Option<crate::workspaces::Workspace>, ApiError> {
    let Some(workspace_id) = request.workspace_id else {
        if request.user_id != actor.user_id && !actor.may_manage_system() {
            return Err(ApiError::Forbidden);
        }
        return Ok(None);
    };
    if request.organization_injection_refs.is_some() || request.user_injection_refs.is_some() {
        return Err(ApiError::BadRequest(
            "an existing workspace preview uses its persisted injection references",
        ));
    }
    let workspace = state.database.get_workspace(workspace_id).await?;
    if !actor.allows(Permission::ReadWorkspace, workspace.organization_id) {
        return Err(ApiError::Forbidden);
    }
    if request
        .organization_id
        .is_some_and(|organization_id| organization_id != workspace.organization_id)
        || request.user_id != workspace.owner_id
    {
        return Err(ApiError::BadRequest(
            "workspace preview organization_id and user_id must match the target workspace",
        ));
    }
    Ok(Some(workspace))
}

fn parse_scope(scope: &str, scope_id: Uuid) -> Result<InjectionScopeRef, ApiError> {
    let scope = InjectionScope::from_database(scope).ok_or(ApiError::BadRequest(
        "scope must be organization, user, or workspace",
    ))?;
    Ok(InjectionScopeRef { scope, scope_id })
}

async fn authorize(
    state: &AppState,
    actor: &Principal,
    scope_ref: InjectionScopeRef,
    write: bool,
    locked: bool,
) -> Result<(), ApiError> {
    let allowed = match scope_ref.scope {
        InjectionScope::Organization => {
            let permission = if write {
                Permission::ManageOrganization
            } else {
                Permission::ReadWorkspace
            };
            actor.allows(permission, scope_ref.scope_id)
                && (!locked || actor.allows(Permission::ManageLockedInjections, scope_ref.scope_id))
        }
        InjectionScope::User => actor.may_manage_system() || actor.user_id == scope_ref.scope_id,
        InjectionScope::Workspace => {
            let workspace = state.database.get_workspace(scope_ref.scope_id).await?;
            actor.allows(
                if write {
                    Permission::ChangeWorkspaceState
                } else {
                    Permission::ReadWorkspace
                },
                workspace.organization_id,
            )
        }
    };
    if !allowed {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}
