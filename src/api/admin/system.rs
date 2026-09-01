use std::sync::Arc;

use axum::{Json, extract::State, http::HeaderMap};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{config::DatabaseMode, storage::JobCounts};

use super::{ApiError, AppState, ErrorEnvelope, require_system_admin};

#[derive(Debug, Serialize, ToSchema)]
pub(in crate::api) struct ScalingResponse {
    database_mode: DatabaseMode,
    configured_replicas: u16,
    schema_version: i64,
    jobs: JobCounts,
}

#[utoipa::path(get, path = "/api/v1/admin/scaling", responses((status = 200, body = ScalingResponse), (status = 403, body = ErrorEnvelope)))]
pub(in crate::api) async fn scaling(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<ScalingResponse>, ApiError> {
    require_system_admin(&state, &headers).await?;
    Ok(Json(ScalingResponse {
        database_mode: state.database.mode(),
        configured_replicas: state.config.replica_count,
        schema_version: state.database.schema_version().await?,
        jobs: state.database.job_counts().await?,
    }))
}
