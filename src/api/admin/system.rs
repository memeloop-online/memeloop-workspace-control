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
    cpu_overcommit_ratio: u16,
    jobs: JobCounts,
}

#[utoipa::path(get, path = "/api/v1/admin/scaling", responses((status = 200, body = ScalingResponse), (status = 403, body = ErrorEnvelope)))]
pub(in crate::api) async fn scaling(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<ScalingResponse>, ApiError> {
    require_system_admin(&state, &headers).await?;
    let settings = state.database.runtime_settings().await?;
    Ok(Json(ScalingResponse {
        database_mode: state.database.mode(),
        configured_replicas: state.config.replica_count,
        schema_version: state.database.schema_version().await?,
        jobs: state.database.job_counts().await?,
        job_worker_concurrency: settings.job_worker_concurrency,
        cpu_overcommit_ratio: settings.cpu_overcommit_ratio,
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct UpdateScalingRequest {
    job_worker_concurrency: Option<u16>,
    cpu_overcommit_ratio: Option<u16>,
}

#[utoipa::path(put, path = "/api/v1/admin/scaling", request_body = UpdateScalingRequest, responses((status = 200, body = ScalingResponse), (status = 400, body = ErrorEnvelope), (status = 403, body = ErrorEnvelope)))]
pub(in crate::api) async fn update_scaling(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<UpdateScalingRequest>,
) -> Result<Json<ScalingResponse>, ApiError> {
    require_system_admin(&state, &headers).await?;
    if request
        .job_worker_concurrency
        .is_some_and(|value| !(1..=crate::storage::MAX_JOB_WORKER_CONCURRENCY).contains(&value))
    {
        return Err(ApiError::BadRequest(
            "job_worker_concurrency must be between 1 and 256",
        ));
    }
    if request
        .cpu_overcommit_ratio
        .is_some_and(|value| !(1..=crate::storage::MAX_CPU_OVERCOMMIT_RATIO).contains(&value))
    {
        return Err(ApiError::BadRequest(
            "cpu_overcommit_ratio must be between 1 and 1000",
        ));
    }
    if request.job_worker_concurrency.is_none() && request.cpu_overcommit_ratio.is_none() {
        return Err(ApiError::BadRequest(
            "at least one scaling setting is required",
        ));
    }
    state
        .database
        .patch_runtime_settings(
            request.job_worker_concurrency,
            request.cpu_overcommit_ratio,
            super::idempotency::unix_timestamp()?,
        )
        .await?;
    scaling(State(state), headers).await
}
