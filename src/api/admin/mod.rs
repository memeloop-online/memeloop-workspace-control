use axum::{http::StatusCode, response::Response};

use crate::storage::{IdempotencyDecision, Principal};

mod api_keys;
#[path = "audit.rs"]
mod audit_api;
mod memberships;
mod quota;
#[path = "settings.rs"]
mod settings;
mod system;
mod users;

pub(super) use api_keys::{
    __path_admin_revoke_api_key, __path_list_user_api_keys, AdminRevokeApiKeyRequest,
    admin_revoke_api_key, list_user_api_keys,
};
pub(super) use audit_api::{__path_audit, audit};
pub(super) use memberships::{
    __path_list_members, __path_remove_membership, __path_upsert_membership, MembershipRequest,
    list_members, remove_membership, upsert_membership,
};
pub(super) use quota::{__path_get_quota, __path_set_quota, get_quota, set_quota};
pub(super) use settings::{
    __path_create_api_key, __path_delete_api_key, __path_get_profile, __path_list_api_keys,
    __path_update_profile, CreateApiKeyRequest, CreatedApiKeyResponse, UpdateUserProfileRequest,
    UserProfileResponse, create_api_key, delete_api_key, get_profile, list_api_keys,
    update_profile,
};
pub(super) use system::{__path_scaling, ScalingResponse, scaling};
pub(super) use users::{
    __path_create_user, __path_list_users_page, __path_update_user, CreateUserRequest, PageQuery,
    UpdateUserRequest, create_user, list_users_page, update_user,
};

pub(super) use super::ErrorEnvelope;

use super::{
    ApiError, AppState,
    auth::principal,
    idempotency,
    idempotency::{IDEMPOTENCY_TTL_SECONDS, replay_response},
};

async fn require_system_admin(
    state: &AppState,
    headers: &axum::http::HeaderMap,
) -> Result<Principal, ApiError> {
    let actor = super::auth::principal(state, headers).await?;
    if !actor.may_manage_system() {
        return Err(ApiError::Forbidden);
    }
    Ok(actor)
}

async fn reserve(
    state: &AppState,
    scope: &str,
    key: &str,
    request_hash: &str,
    now: i64,
) -> Result<Option<Response>, ApiError> {
    match state
        .database
        .begin_idempotency(scope, key, request_hash, now, now + IDEMPOTENCY_TTL_SECONDS)
        .await?
    {
        IdempotencyDecision::Reserved => Ok(None),
        IdempotencyDecision::Replay(replay) => replay_response(replay).map(Some),
        IdempotencyDecision::Conflict => Err(ApiError::IdempotencyConflict),
        IdempotencyDecision::InProgress => Err(ApiError::IdempotencyInProgress),
    }
}

async fn finish_empty(
    state: &AppState,
    scope: &str,
    key: &str,
    request_hash: &str,
) -> Result<Response, ApiError> {
    state
        .database
        .finish_idempotency(
            scope,
            key,
            request_hash,
            StatusCode::NO_CONTENT.as_u16(),
            "",
        )
        .await?;
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(axum::body::Body::empty())
        .map_err(|_| ApiError::BadRequest("response could not be built"))
}
