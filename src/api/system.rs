use std::{sync::Arc, time::Duration};

use axum::{Json, extract::State, http::StatusCode};
use serde::Serialize;
use utoipa::ToSchema;

use crate::config::{DatabaseMode, InstallationId};

use super::AppState;

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct HealthResponse {
    status: &'static str,
}

#[utoipa::path(
    get,
    path = "/livez",
    responses((status = 200, description = "Process can serve HTTP", body = HealthResponse))
)]
pub(super) async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

#[utoipa::path(
    get,
    path = "/readyz",
    responses(
        (status = 200, description = "Authoritative database is reachable", body = HealthResponse),
        (status = 503, description = "Authoritative database is unavailable")
    )
)]
pub(super) async fn ready(
    State(state): State<Arc<AppState>>,
) -> Result<Json<HealthResponse>, StatusCode> {
    tokio::time::timeout(Duration::from_secs(2), state.database.ping())
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(HealthResponse { status: "ok" }))
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct SystemInfoResponse {
    installation_id: InstallationId,
    api_version: &'static str,
    database_mode: DatabaseMode,
}

#[utoipa::path(
    get,
    path = "/api/v1/system/info",
    responses((status = 200, description = "Non-sensitive installation metadata", body = SystemInfoResponse))
)]
pub(super) async fn system_info(State(state): State<Arc<AppState>>) -> Json<SystemInfoResponse> {
    Json(SystemInfoResponse {
        installation_id: state.config.installation_id.clone(),
        api_version: "v1",
        database_mode: state.database.mode(),
    })
}
