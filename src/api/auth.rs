use std::sync::Arc;

use axum::{Json, extract::State, http::HeaderMap};

use crate::storage::Principal;

use super::{ApiError, AppState};

pub(super) mod authentik;

pub(super) async fn principal(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Principal, ApiError> {
    optional_principal(state, headers)
        .await?
        .ok_or(ApiError::Unauthorized)
}

pub(super) async fn optional_principal(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<Principal>, ApiError> {
    if let Some(authorization) = headers.get(axum::http::header::AUTHORIZATION) {
        let token = authorization
            .to_str()
            .ok()
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or(ApiError::Unauthorized)?;
        return Ok(state.database.authenticate(token).await?);
    }
    if let Some(user_id) = authentik::browser_user(headers).await? {
        return state
            .database
            .browser_principal(user_id)
            .await?
            .map(Some)
            .ok_or(ApiError::Forbidden);
    }
    Ok(None)
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
