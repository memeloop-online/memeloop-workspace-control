use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    auth::{ApiKeyScope, Permission, Role},
    storage::{IdempotencyDecision, InitialUserCommand, UserPage, UserSummary},
};

use super::{
    ApiError, AppState, ErrorEnvelope,
    idempotency::{hash, idempotency_key, json_response, unix_timestamp},
    require_system_admin,
};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(in crate::api) struct CreateUserRequest {
    display_name: String,
    token: String,
    #[serde(default)]
    system_admin: bool,
    /// Omitted by old clients: use the safe role-specific default below.
    #[serde(default)]
    scopes: Option<Vec<ApiKeyScope>>,
    /// Unix seconds. Omitted by old clients: defaults to 30 days.
    #[serde(default)]
    expires_at: Option<i64>,
    /// Optionally place the new user in an organization as part of creation.
    #[serde(default)]
    organization_id: Option<Uuid>,
    /// Required with `organization_id`; organization roles never accept `system_admin`.
    #[serde(default)]
    organization_role: Option<Role>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub(in crate::api) struct PageQuery {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    pub search: Option<String>,
    pub organization_id: Option<Uuid>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub(in crate::api) struct UpdateUserRequest {
    pub display_name: Option<String>,
    pub system_admin: Option<bool>,
    pub disabled: Option<bool>,
}

#[utoipa::path(get, path = "/api/v1/admin/users", params(PageQuery), responses((status = 200, body = UserPage), (status = 403, body = ErrorEnvelope)))]
pub(in crate::api) async fn list_users_page(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
) -> Result<Json<UserPage>, ApiError> {
    require_system_admin(&state, &headers).await?;
    Ok(Json(
        state
            .database
            .list_users_page(
                query.limit,
                query.cursor.as_deref(),
                query.search.as_deref(),
                query.organization_id,
            )
            .await?,
    ))
}

#[utoipa::path(
    put,
    path = "/api/v1/admin/users/{user_id}",
    request_body = UpdateUserRequest,
    params(("user_id" = Uuid, Path)),
    responses((status = 200, body = UserSummary), (status = 403, body = ErrorEnvelope), (status = 409, body = ErrorEnvelope))
)]
pub(in crate::api) async fn update_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(user_id): Path<Uuid>,
    Json(request): Json<UpdateUserRequest>,
) -> Result<Json<UserSummary>, ApiError> {
    let actor = require_system_admin(&state, &headers).await?;
    if actor.user_id == user_id
        && (request.disabled == Some(true) || request.system_admin == Some(false))
    {
        return Err(ApiError::BadRequest(
            "you cannot disable or demote your own system administrator account",
        ));
    }
    if request.display_name.is_none()
        && request.system_admin.is_none()
        && request.disabled.is_none()
    {
        return Err(ApiError::BadRequest("at least one user field is required"));
    }
    let summary = state
        .database
        .update_user(
            user_id,
            request.display_name.as_deref(),
            request.system_admin,
            request.disabled,
        )
        .await?;
    state
        .database
        .record_audit(
            Some(actor.user_id),
            None,
            None,
            "user.update",
            serde_json::json!({"user_id": user_id}),
            unix_timestamp()?,
        )
        .await?;
    Ok(Json(summary))
}

#[utoipa::path(post, path = "/api/v1/admin/users", request_body = CreateUserRequest, params(("Idempotency-Key" = String, Header)), responses((status = 201, body = UserSummary), (status = 403, body = ErrorEnvelope)))]
pub(in crate::api) async fn create_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CreateUserRequest>,
) -> Result<Response, ApiError> {
    let actor = require_system_admin(&state, &headers).await?;
    let key = idempotency_key(&headers)?;
    let now = unix_timestamp()?;
    let command = initial_user_command(&actor, &request, now)?;
    let request_hash = hash(&request)?;
    let scope = format!("{}:admin-create-user", actor.user_id);
    match state
        .database
        .begin_idempotency(
            &scope,
            key,
            &request_hash,
            now,
            now + super::super::idempotency::IDEMPOTENCY_TTL_SECONDS,
        )
        .await?
    {
        IdempotencyDecision::Replay(replay) => {
            return super::super::idempotency::replay_response(replay);
        }
        IdempotencyDecision::Conflict => return Err(ApiError::IdempotencyConflict),
        IdempotencyDecision::InProgress => return Err(ApiError::IdempotencyInProgress),
        IdempotencyDecision::Reserved => {}
    }
    let principal = match state
        .database
        .create_user_with_initial_key_and_membership(command)
        .await
    {
        Ok(principal) => principal,
        Err(error) => {
            state
                .database
                .abandon_idempotency(&scope, key, &request_hash)
                .await?;
            return Err(error.into());
        }
    };
    let membership_role = principal
        .memberships
        .first()
        .map(|membership| membership.role);
    let summary = UserSummary {
        id: principal.user_id,
        display_name: principal.display_name,
        system_admin: principal.system_admin,
        disabled: false,
        created_at: now,
        membership_role: Some(membership_role),
    };
    state
        .database
        .record_audit(
            Some(actor.user_id),
            None,
            None,
            "user.create",
            serde_json::json!({"user_id": summary.id, "system_admin": summary.system_admin}),
            now,
        )
        .await?;
    let response_json = serde_json::to_string(&summary)
        .map_err(|_| ApiError::BadRequest("response serialization failed"))?;
    state
        .database
        .finish_idempotency(
            &scope,
            key,
            &request_hash,
            StatusCode::CREATED.as_u16(),
            &response_json,
        )
        .await?;
    json_response(StatusCode::CREATED, response_json)
}

fn initial_user_command<'a>(
    actor: &crate::storage::Principal,
    request: &'a CreateUserRequest,
    now: i64,
) -> Result<InitialUserCommand<'a>, ApiError> {
    let membership = requested_membership(actor, request)?;
    let scopes = request
        .scopes
        .clone()
        .unwrap_or_else(|| ApiKeyScope::initial_key_defaults(request.system_admin));
    let expires_at = request
        .expires_at
        .unwrap_or_else(|| now + INITIAL_KEY_LIFETIME_SECONDS);
    if !super::settings::actor_may_grant(&actor.api_key_scopes, &scopes)
        || actor
            .api_key_expires_at
            .is_some_and(|limit| expires_at > limit)
    {
        return Err(ApiError::Forbidden);
    }
    // Validate before reserving the idempotency key so malformed requests don't
    // leave a reservation behind. The storage path validates again atomically.
    crate::storage::validate_api_key_policy(scopes.clone(), Some(expires_at), now)?;
    Ok(InitialUserCommand {
        display_name: &request.display_name,
        token: &request.token,
        system_admin: request.system_admin,
        scopes,
        expires_at,
        membership,
        now,
    })
}

fn requested_membership(
    actor: &crate::storage::Principal,
    request: &CreateUserRequest,
) -> Result<Option<(Uuid, Role)>, ApiError> {
    match (request.organization_id, request.organization_role) {
        (Some(organization_id), Some(role @ (Role::OrganizationAdmin | Role::Member))) => {
            if actor.allows(Permission::ManageMembers, organization_id) {
                Ok(Some((organization_id, role)))
            } else {
                Err(ApiError::Forbidden)
            }
        }
        (Some(_), Some(Role::SystemAdmin)) => Err(ApiError::BadRequest(
            "organization membership cannot grant system_admin",
        )),
        (Some(_), None) | (None, Some(_)) => Err(ApiError::BadRequest(
            "organization_id and organization_role must be supplied together",
        )),
        (None, None) => Ok(None),
    }
}

const INITIAL_KEY_LIFETIME_SECONDS: i64 = 30 * 24 * 60 * 60;
