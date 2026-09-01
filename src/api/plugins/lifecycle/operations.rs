use axum::{
    http::{HeaderMap, StatusCode},
    response::Response,
};
use serde::Serialize;

use super::super::super::{
    ApiError, AppState,
    auth::principal,
    idempotency::{IDEMPOTENCY_TTL_SECONDS, json_response, replay_response},
};
use crate::storage::IdempotencyDecision;

pub(super) async fn system_admin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<crate::storage::Principal, ApiError> {
    let actor = principal(state, headers).await?;
    if !actor.may_manage_system() {
        return Err(ApiError::Forbidden);
    }
    Ok(actor)
}
pub(super) async fn reserve(
    state: &AppState,
    scope: &str,
    key: &str,
    request_hash: &str,
    now: i64,
) -> Result<Option<Response>, ApiError> {
    Ok(
        match state
            .database
            .begin_idempotency(scope, key, request_hash, now, now + IDEMPOTENCY_TTL_SECONDS)
            .await?
        {
            IdempotencyDecision::Replay(value) => Some(replay_response(value)?),
            IdempotencyDecision::Conflict => return Err(ApiError::IdempotencyConflict),
            IdempotencyDecision::InProgress => return Err(ApiError::IdempotencyInProgress),
            IdempotencyDecision::Reserved => None,
        },
    )
}
pub(super) async fn abandon(
    state: &AppState,
    scope: &str,
    key: &str,
    request_hash: &str,
) -> Result<(), ApiError> {
    state
        .database
        .abandon_idempotency(scope, key, request_hash)
        .await?;
    Ok(())
}
pub(super) async fn finish_json<T: Serialize>(
    state: &AppState,
    scope: &str,
    key: &str,
    request_hash: &str,
    status: StatusCode,
    value: &T,
) -> Result<Response, ApiError> {
    let body = if status == StatusCode::NO_CONTENT {
        String::new()
    } else {
        serde_json::to_string(value)
            .map_err(|_| ApiError::PluginDistribution("plugin_package_invalid"))?
    };
    state
        .database
        .finish_idempotency(scope, key, request_hash, status.as_u16(), &body)
        .await?;
    if status == StatusCode::NO_CONTENT {
        Ok(Response::builder()
            .status(status)
            .body(axum::body::Body::empty())
            .map_err(|_| ApiError::PluginDistribution("plugin_package_invalid"))?)
    } else {
        json_response(status, body)
    }
}
