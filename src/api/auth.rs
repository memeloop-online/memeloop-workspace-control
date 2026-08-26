use std::sync::Arc;

use axum::{Json, extract::State, http::HeaderMap};

use crate::storage::Principal;

use super::{ApiError, AppState};

pub(super) async fn principal(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Principal, ApiError> {
    let authorization = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or(ApiError::Unauthorized)?;
    let token = authorization
        .strip_prefix("Bearer ")
        .ok_or(ApiError::Unauthorized)?;
    state
        .database
        .authenticate(token)
        .await?
        .ok_or(ApiError::Unauthorized)
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
