use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
    response::Response,
};
use uuid::Uuid;

use crate::{auth::Permission, quota::Resources};

use super::{
    ApiError, AppState, ErrorEnvelope, finish_empty,
    idempotency::{hash, idempotency_key, unix_timestamp},
    principal, reserve,
};

#[utoipa::path(get, path = "/api/v1/organizations/{organization_id}/quota", responses((status = 200, body = Option<Resources>), (status = 403, body = ErrorEnvelope)))]
pub(in crate::api) async fn get_quota(
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

#[utoipa::path(put, path = "/api/v1/organizations/{organization_id}/quota", request_body = Resources, params(("Idempotency-Key" = String, Header)), responses((status = 204), (status = 403, body = ErrorEnvelope)))]
pub(in crate::api) async fn set_quota(
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
