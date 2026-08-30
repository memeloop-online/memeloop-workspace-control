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
    auth::{Permission, Role},
    config::DatabaseMode,
    quota::Resources,
    storage::{IdempotencyDecision, JobCounts, UserSummary},
};

#[path = "admin/audit.rs"]
mod audit_api;
#[path = "admin/settings.rs"]
mod settings;

pub(super) use audit_api::{__path_audit, audit};
pub(super) use settings::{
    __path_create_api_key, __path_delete_api_key, __path_get_profile, __path_list_api_keys,
    __path_update_profile, CreateApiKeyRequest, CreatedApiKeyResponse, UpdateUserProfileRequest,
    UserProfileResponse, create_api_key, delete_api_key, get_profile, list_api_keys,
    update_profile,
};

use super::{
    ApiError, AppState,
    auth::principal,
    idempotency::{
        IDEMPOTENCY_TTL_SECONDS, hash, idempotency_key, json_response, replay_response,
        unix_timestamp,
    },
};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(super) struct CreateUserRequest {
    display_name: String,
    token: String,
    #[serde(default)]
    system_admin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(super) struct MembershipRequest {
    role: Role,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct ScalingResponse {
    database_mode: DatabaseMode,
    configured_replicas: u16,
    schema_version: i64,
    jobs: JobCounts,
}

#[utoipa::path(get, path = "/api/v1/admin/users", responses((status = 200, body = [UserSummary]), (status = 403, body = super::ErrorEnvelope)))]
pub(super) async fn list_users(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<UserSummary>>, ApiError> {
    require_system_admin(&state, &headers).await?;
    Ok(Json(state.database.list_users().await?))
}

#[utoipa::path(post, path = "/api/v1/admin/users", request_body = CreateUserRequest, params(("Idempotency-Key" = String, Header)), responses((status = 201, body = UserSummary), (status = 403, body = super::ErrorEnvelope)))]
pub(super) async fn create_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CreateUserRequest>,
) -> Result<Response, ApiError> {
    let actor = require_system_admin(&state, &headers).await?;
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&request)?;
    let scope = format!("{}:admin-create-user", actor.user_id);
    let now = unix_timestamp()?;
    match state
        .database
        .begin_idempotency(
            &scope,
            key,
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
    let principal = match state
        .database
        .create_user(
            &request.display_name,
            &request.token,
            request.system_admin,
            now,
        )
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
    let summary = UserSummary {
        id: principal.user_id,
        display_name: principal.display_name,
        system_admin: principal.system_admin,
        disabled: false,
        created_at: now,
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

#[utoipa::path(put, path = "/api/v1/organizations/{organization_id}/members/{user_id}", request_body = MembershipRequest, params(("Idempotency-Key" = String, Header)), responses((status = 204), (status = 403, body = super::ErrorEnvelope)))]
pub(super) async fn upsert_membership(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((organization_id, user_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<MembershipRequest>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.allows(Permission::ManageMembers, organization_id) {
        return Err(ApiError::Forbidden);
    }
    if request.role == Role::SystemAdmin {
        return Err(ApiError::BadRequest(
            "organization membership cannot grant system_admin",
        ));
    }
    let key = idempotency_key(&headers)?;
    let request_hash = hash(
        &serde_json::json!({"organization_id": organization_id, "user_id": user_id, "role": request.role}),
    )?;
    let scope = format!("{}:upsert-membership", actor.user_id);
    let now = unix_timestamp()?;
    if let Some(response) = reserve(&state, &scope, key, &request_hash, now).await? {
        return Ok(response);
    }
    state
        .database
        .upsert_membership(organization_id, user_id, request.role, now)
        .await?;
    state
        .database
        .enqueue_injection_reconciles(
            crate::storage::InjectionScopeRef {
                scope: crate::injections::InjectionScope::Organization,
                scope_id: organization_id,
            },
            now,
        )
        .await?;
    state
        .database
        .record_audit(
            Some(actor.user_id),
            Some(organization_id),
            None,
            "membership.upsert",
            serde_json::json!({"user_id": user_id, "role": request.role}),
            now,
        )
        .await?;
    finish_empty(&state, &scope, key, &request_hash).await
}

#[utoipa::path(delete, path = "/api/v1/organizations/{organization_id}/members/{user_id}", params(("Idempotency-Key" = String, Header)), responses((status = 204), (status = 403, body = super::ErrorEnvelope)))]
pub(super) async fn remove_membership(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((organization_id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.allows(Permission::ManageMembers, organization_id) {
        return Err(ApiError::Forbidden);
    }
    let key = idempotency_key(&headers)?;
    let request_hash =
        hash(&serde_json::json!({"organization_id": organization_id, "user_id": user_id}))?;
    let scope = format!("{}:remove-membership", actor.user_id);
    let now = unix_timestamp()?;
    if let Some(response) = reserve(&state, &scope, key, &request_hash, now).await? {
        return Ok(response);
    }
    state
        .database
        .remove_membership(organization_id, user_id)
        .await?;
    state
        .database
        .enqueue_injection_reconciles(
            crate::storage::InjectionScopeRef {
                scope: crate::injections::InjectionScope::Organization,
                scope_id: organization_id,
            },
            now,
        )
        .await?;
    state
        .database
        .record_audit(
            Some(actor.user_id),
            Some(organization_id),
            None,
            "membership.remove",
            serde_json::json!({"user_id": user_id}),
            now,
        )
        .await?;
    finish_empty(&state, &scope, key, &request_hash).await
}

#[utoipa::path(get, path = "/api/v1/organizations/{organization_id}/quota", responses((status = 200, body = Option<Resources>), (status = 403, body = super::ErrorEnvelope)))]
pub(super) async fn get_quota(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(organization_id): Path<Uuid>,
) -> Result<Json<Option<Resources>>, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.allows(Permission::ReadWorkspace, organization_id) {
        return Err(ApiError::Forbidden);
    }
    Ok(Json(
        state
            .database
            .get_organization_quota(organization_id)
            .await?,
    ))
}

#[utoipa::path(put, path = "/api/v1/organizations/{organization_id}/quota", request_body = Resources, params(("Idempotency-Key" = String, Header)), responses((status = 204), (status = 403, body = super::ErrorEnvelope)))]
pub(super) async fn set_quota(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(organization_id): Path<Uuid>,
    Json(resources): Json<Resources>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.allows(Permission::ManageOrganization, organization_id) {
        return Err(ApiError::Forbidden);
    }
    let key = idempotency_key(&headers)?;
    let request_hash =
        hash(&serde_json::json!({"organization_id": organization_id, "resources": resources}))?;
    let scope = format!("{}:set-quota", actor.user_id);
    let now = unix_timestamp()?;
    if let Some(response) = reserve(&state, &scope, key, &request_hash, now).await? {
        return Ok(response);
    }
    state
        .database
        .set_organization_quota(organization_id, resources, now)
        .await?;
    state
        .database
        .record_audit(
            Some(actor.user_id),
            Some(organization_id),
            None,
            "quota.set",
            serde_json::to_value(resources)
                .map_err(|_| ApiError::BadRequest("quota cannot be serialized"))?,
            now,
        )
        .await?;
    finish_empty(&state, &scope, key, &request_hash).await
}

#[utoipa::path(get, path = "/api/v1/admin/scaling", responses((status = 200, body = ScalingResponse), (status = 403, body = super::ErrorEnvelope)))]
pub(super) async fn scaling(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<ScalingResponse>, ApiError> {
    require_system_admin(&state, &headers).await?;
    Ok(Json(ScalingResponse {
        database_mode: state.database.mode(),
        configured_replicas: state.config.replica_count,
        schema_version: state.database.schema_version().await?,
        jobs: state.database.job_counts().await?,
    }))
}

async fn require_system_admin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<crate::storage::Principal, ApiError> {
    let actor = principal(state, headers).await?;
    if !actor.system_admin {
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
