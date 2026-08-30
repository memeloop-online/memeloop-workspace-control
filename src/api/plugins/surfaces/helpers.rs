use axum::{
    http::{HeaderMap, StatusCode, header},
    response::Response,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::super::super::{
    ApiError, AppState,
    idempotency::{IDEMPOTENCY_TTL_SECONDS, json_response, replay_response},
};
use crate::storage::{IdempotencyDecision, PluginUiSession};

pub(super) async fn validate_session_package(
    state: &AppState,
    session: &PluginUiSession,
    plugin_id: &str,
) -> Result<(), ApiError> {
    if session.plugin_id != plugin_id {
        return Err(ApiError::PluginDistribution("plugin_ui_session_invalid"));
    }
    let current = state
        .database
        .list_plugin_packages()
        .await?
        .into_iter()
        .find(|item| {
            item.plugin_id == plugin_id
                && item.enabled
                && item.package_digest == session.package_digest
        });
    if current.is_none() {
        return Err(ApiError::PluginDistribution("plugin_ui_session_invalid"));
    }
    Ok(())
}
pub(super) fn random_token() -> Result<String, ApiError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|_| ApiError::Storage(crate::storage::StorageError::RandomSource))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
pub(super) fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
pub(super) fn cookie_name(id: Uuid) -> String {
    format!("mwc_plugin_{}", id.simple())
}
pub(super) fn cookie_value(headers: &HeaderMap, id: Uuid) -> Option<&str> {
    let expected = cookie_name(id);
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(name, value)| (name == expected).then_some(value))
}
pub(super) fn safe_route_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}
pub(super) fn safe_method(value: &str) -> bool {
    matches!(value, "GET" | "POST" | "PUT" | "PATCH" | "DELETE")
}
pub(super) fn safe_path(value: &str) -> bool {
    value.len() <= 512
        && !value.starts_with('/')
        && !value.contains(['\\', '\0'])
        && value.split('/').all(|part| {
            !part.is_empty() && part != "." && part != ".." && !part.chars().any(char::is_control)
        })
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
pub(super) async fn finish<T: Serialize>(
    state: &AppState,
    scope: &str,
    key: &str,
    request_hash: &str,
    value: &T,
) -> Result<Response, ApiError> {
    let body = serde_json::to_string(value)
        .map_err(|_| ApiError::PluginDistribution("plugin_package_invalid"))?;
    state
        .database
        .finish_idempotency(scope, key, request_hash, StatusCode::OK.as_u16(), &body)
        .await?;
    json_response(StatusCode::OK, body)
}
