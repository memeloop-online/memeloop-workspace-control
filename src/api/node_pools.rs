use std::sync::Arc;

use axum::{Json, extract::State, http::HeaderMap};

use crate::storage::AvailableNodePool;

use super::{ApiError, AppState, auth::principal};

#[utoipa::path(
    get,
    path = "/api/v1/node-pools",
    responses(
        (status = 200, body = [AvailableNodePool]),
        (status = 401, body = super::ErrorEnvelope)
    )
)]
pub(super) async fn list_available(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<AvailableNodePool>>, ApiError> {
    principal(&state, &headers).await?;
    Ok(Json(state.database.list_available_node_pools().await?))
}
