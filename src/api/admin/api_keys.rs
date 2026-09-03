use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    api::{ApiError, AppState, ErrorEnvelope, idempotency::unix_timestamp},
    storage::{ApiKeyListStatus, ApiKeyPage},
};

use super::require_system_admin;

/// Paginated administrator view of a user's API keys. The result contains
/// summaries only; plaintext tokens and token hashes are never exposed.
#[derive(Debug, Deserialize, IntoParams)]
pub(in crate::api) struct AdminApiKeyPageQuery {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    pub status: Option<ApiKeyListStatus>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub(in crate::api) struct AdminRevokeApiKeyRequest {
    /// Required operator justification, retained in the audit event metadata.
    pub reason: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/users/{user_id}/api-keys",
    params(("user_id" = Uuid, Path), AdminApiKeyPageQuery),
    responses(
        (status = 200, body = ApiKeyPage),
        (status = 400, body = ErrorEnvelope),
        (status = 401, body = ErrorEnvelope),
        (status = 403, body = ErrorEnvelope)
    )
)]
pub(in crate::api) async fn list_user_api_keys(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(user_id): Path<Uuid>,
    Query(query): Query<AdminApiKeyPageQuery>,
) -> Result<Json<ApiKeyPage>, ApiError> {
    let actor = require_system_admin(&state, &headers).await?;
    if !actor.may_manage_api_keys() {
        return Err(ApiError::Forbidden);
    }
    Ok(Json(
        state
            .database
            .list_api_keys_page(
                user_id,
                query.status.unwrap_or(ApiKeyListStatus::Active),
                query.limit,
                query.cursor.as_deref(),
            )
            .await?,
    ))
}

#[utoipa::path(
    delete,
    path = "/api/v1/admin/users/{user_id}/api-keys/{key_id}",
    request_body = AdminRevokeApiKeyRequest,
    params(("user_id" = Uuid, Path), ("key_id" = Uuid, Path)),
    responses(
        (status = 204),
        (status = 400, body = ErrorEnvelope),
        (status = 401, body = ErrorEnvelope),
        (status = 403, body = ErrorEnvelope)
    )
)]
pub(in crate::api) async fn admin_revoke_api_key(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((user_id, key_id)): Path<(Uuid, Uuid)>,
    request: Result<Json<AdminRevokeApiKeyRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let actor = require_system_admin(&state, &headers).await?;
    if !actor.may_manage_api_keys() {
        return Err(ApiError::Forbidden);
    }
    if actor.user_id == user_id {
        return Err(ApiError::BadRequest(
            "use the personal API-key endpoint when revoking your own key",
        ));
    }
    let Json(request) = request.map_err(|_| ApiError::BadRequest("a JSON reason is required"))?;
    let reason = validate_reason(&request.reason)?;
    state
        .database
        .admin_revoke_api_key(actor.user_id, user_id, key_id, reason, unix_timestamp()?)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

fn validate_reason(reason: &str) -> Result<&str, ApiError> {
    let reason = reason.trim();
    if reason.is_empty() || reason.chars().count() > 500 {
        return Err(ApiError::BadRequest(
            "reason must contain between 1 and 500 characters",
        ));
    }
    Ok(reason)
}

#[cfg(test)]
mod tests {
    use super::validate_reason;

    #[test]
    fn revoke_reason_must_be_nonempty_and_bounded_in_characters() {
        assert!(validate_reason("  operator request  ").is_ok());
        assert!(validate_reason(" \n\t ").is_err());
        assert!(validate_reason(&"中".repeat(500)).is_ok());
        assert!(validate_reason(&"中".repeat(501)).is_err());
    }
}
