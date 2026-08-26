use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    body::Body,
    http::{HeaderMap, StatusCode, header::CONTENT_TYPE},
    response::Response,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::ApiError;

pub(super) const IDEMPOTENCY_TTL_SECONDS: i64 = 86_400;

pub(super) fn replay_response(
    replay: crate::storage::IdempotencyReplay,
) -> Result<Response, ApiError> {
    let status = StatusCode::from_u16(replay.status_code)
        .map_err(|_| ApiError::BadRequest("stored response status is invalid"))?;
    json_response(status, replay.response_json)
}

pub(super) fn json_response(status: StatusCode, body: String) -> Result<Response, ApiError> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .map_err(|_| ApiError::BadRequest("response could not be built"))
}

pub(super) fn idempotency_key(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get("Idempotency-Key")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .ok_or(ApiError::MissingIdempotencyKey)
}

pub(super) fn hash(value: &impl Serialize) -> Result<String, ApiError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| ApiError::BadRequest("request cannot be canonicalized"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub(super) fn unix_timestamp() -> Result<i64, ApiError> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ApiError::BadRequest("system clock is before Unix epoch"))?
        .as_secs();
    i64::try_from(seconds).map_err(|_| ApiError::BadRequest("system clock overflow"))
}
