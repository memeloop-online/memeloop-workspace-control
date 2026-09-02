use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::Response,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    auth::{Permission, Role},
    storage::MembershipPage,
};

use super::{
    ApiError, AppState, ErrorEnvelope, PageQuery, finish_empty,
    idempotency::{hash, idempotency_key, unix_timestamp},
    principal, reserve,
};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(in crate::api) struct MembershipRequest {
    role: Role,
}

#[utoipa::path(
    get,
    path = "/api/v1/organizations/{organization_id}/members",
    params(("organization_id" = Uuid, Path), PageQuery),
    responses((status = 200, body = MembershipPage), (status = 403, body = ErrorEnvelope))
)]
pub(in crate::api) async fn list_members(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(organization_id): Path<Uuid>,
    Query(query): Query<PageQuery>,
) -> Result<Json<MembershipPage>, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.allows(Permission::ManageMembers, organization_id) {
        return Err(ApiError::Forbidden);
    }
    Ok(Json(
        state
            .database
            .list_members_page(
                organization_id,
                query.limit,
                query.cursor.as_deref(),
                query.search.as_deref(),
            )
            .await?,
    ))
}

#[utoipa::path(put, path = "/api/v1/organizations/{organization_id}/members/{user_id}", request_body = MembershipRequest, params(("Idempotency-Key" = String, Header)), responses((status = 204), (status = 403, body = ErrorEnvelope), (status = 409, body = ErrorEnvelope)))]
pub(in crate::api) async fn upsert_membership(
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
    let result = async {
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
        Ok::<(), ApiError>(())
    }
    .await;
    if let Err(error) = result {
        state
            .database
            .abandon_idempotency(&scope, key, &request_hash)
            .await?;
        return Err(error);
    }
    finish_empty(&state, &scope, key, &request_hash).await
}

#[utoipa::path(delete, path = "/api/v1/organizations/{organization_id}/members/{user_id}", params(("Idempotency-Key" = String, Header)), responses((status = 204), (status = 403, body = ErrorEnvelope), (status = 409, body = ErrorEnvelope)))]
pub(in crate::api) async fn remove_membership(
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
    let result = async {
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
        Ok::<(), ApiError>(())
    }
    .await;
    if let Err(error) = result {
        state
            .database
            .abandon_idempotency(&scope, key, &request_hash)
            .await?;
        return Err(error);
    }
    finish_empty(&state, &scope, key, &request_hash).await
}
