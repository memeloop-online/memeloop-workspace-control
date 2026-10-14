use std::sync::Arc;

use axum::{Json, extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{config::DatabaseMode, storage::JobCounts};

use super::{ApiError, AppState, ErrorEnvelope, require_system_admin};

#[derive(Debug, Serialize, ToSchema)]
pub(in crate::api) struct ScalingResponse {
    database_mode: DatabaseMode,
    configured_replicas: u16,
    schema_version: i64,
    job_worker_concurrency: u16,
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
        job_worker_concurrency: state.database.job_worker_concurrency().await?,
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct UpdateScalingRequest {
    job_worker_concurrency: u16,
}

#[utoipa::path(put, path = "/api/v1/admin/scaling", request_body = UpdateScalingRequest, responses((status = 200, body = ScalingResponse), (status = 400, body = ErrorEnvelope), (status = 403, body = ErrorEnvelope)))]
pub(in crate::api) async fn update_scaling(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<UpdateScalingRequest>,
) -> Result<Json<ScalingResponse>, ApiError> {
    require_system_admin(&state, &headers).await?;
    if !(1..=crate::storage::MAX_JOB_WORKER_CONCURRENCY).contains(&request.job_worker_concurrency) {
        return Err(ApiError::BadRequest(
            "job_worker_concurrency must be between 1 and 256",
        ));
    }
    state
        .database
        .set_job_worker_concurrency(
            request.job_worker_concurrency,
            super::idempotency::unix_timestamp()?,
        )
        .await?;
    scaling(State(state), headers).await
}
