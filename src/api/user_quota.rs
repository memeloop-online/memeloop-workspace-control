use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use uuid::Uuid;

use crate::{quota::Resources, storage::IdempotencyDecision};

use super::{
    ApiError, AppState,
    auth::principal,
    idempotency::{
        IDEMPOTENCY_TTL_SECONDS, hash, idempotency_key, replay_response, unix_timestamp,
    },
};

#[utoipa::path(get, path = "/api/v1/admin/users/{user_id}/quota", responses((status = 200, body = Option<Resources>), (status = 403, body = super::ErrorEnvelope)))]
pub(super) async fn get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(user_id): Path<Uuid>,
) -> Result<Json<Option<Resources>>, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.system_admin && actor.user_id != user_id {
        return Err(ApiError::Forbidden);
    }
    Ok(Json(state.database.get_user_quota(user_id).await?))
}

#[utoipa::path(put, path = "/api/v1/admin/users/{user_id}/quota", request_body = Resources, params(("Idempotency-Key" = String, Header)), responses((status = 204), (status = 403, body = super::ErrorEnvelope)))]
pub(super) async fn set(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(user_id): Path<Uuid>,
    Json(resources): Json<Resources>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.system_admin {
        return Err(ApiError::Forbidden);
    }
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&serde_json::json!({"user_id": user_id, "resources": resources}))?;
    let scope = format!("{}:set-user-quota", actor.user_id);
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
    state
        .database
        .set_user_quota(user_id, resources, now)
        .await?;
    state
        .database
        .record_audit(
            Some(actor.user_id),
            None,
            None,
            "user_quota.set",
            serde_json::json!({"user_id": user_id, "resources": resources}),
            now,
        )
        .await?;
    state
        .database
        .finish_idempotency(
            &scope,
            key,
            &request_hash,
            StatusCode::NO_CONTENT.as_u16(),
            "",
        )
        .await?;
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(axum::body::Body::empty())
        .map_err(|_| ApiError::BadRequest("response could not be built"))
}
