use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, header},
};

use crate::storage::Principal;

use super::{ApiError, AppState};

pub(super) mod oidc;
pub(super) mod oidc_provider;

pub(super) async fn principal(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Principal, ApiError> {
    optional_principal(state, headers)
        .await?
        .ok_or(ApiError::Unauthorized)
}

pub(super) fn bearer(headers: &HeaderMap) -> Result<Option<&str>, ApiError> {
    headers
        .get(header::AUTHORIZATION)
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|value| value.strip_prefix("Bearer "))
                .ok_or(ApiError::Unauthorized)
        })
        .transpose()
}

pub(super) async fn optional_principal(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<Principal>, ApiError> {
    if let Some(token) = bearer(headers)? {
        return Ok(state.database.authenticate(token).await?);
    }
    oidc::principal(state, headers).await
}

#[utoipa::path(
    get,
    path = "/api/v1/me",
    responses(
        (status = 200, description = "Authenticated principal", body = Principal),
        (status = 401, description = "Authentication required", body = super::ErrorEnvelope)
    )
)]
pub(super) async fn me(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Principal>, ApiError> {
    Ok(Json(principal(&state, &headers).await?))
}
