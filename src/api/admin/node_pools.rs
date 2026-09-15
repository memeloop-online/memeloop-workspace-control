use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};

use crate::storage::{IdempotencyDecision, NodePool, PutNodePool};

use super::super::{
    ApiError, AppState,
    auth::principal,
    idempotency::{hash, idempotency_key, json_response, unix_timestamp},
};

#[utoipa::path(get, path = "/api/v1/admin/node-pools", responses((status = 200, body = [NodePool]), (status = 403, body = super::ErrorEnvelope)))]
pub(crate) async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<NodePool>>, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.may_manage_system() || actor.has_template_restriction() {
        return Err(ApiError::Forbidden);
    }
    Ok(Json(state.database.list_node_pools().await?))
}

#[utoipa::path(put, path = "/api/v1/admin/node-pools/{name}", request_body = PutNodePool, params(("name" = String, Path), ("Idempotency-Key" = String, Header)), responses((status = 200, body = NodePool), (status = 403, body = super::ErrorEnvelope), (status = 409, body = super::ErrorEnvelope)))]
pub(crate) async fn put(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(command): Json<PutNodePool>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.may_manage_system() || actor.has_template_restriction() {
        return Err(ApiError::Forbidden);
    }
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&(name.as_str(), &command))?;
    let scope = format!("{}:put-node-pool", actor.user_id);
    let now = unix_timestamp()?;
    match state
        .database
        .begin_idempotency(&scope, key, &request_hash, now, now + 86_400)
        .await?
    {
        IdempotencyDecision::Replay(replay) => {
            return super::super::idempotency::replay_response(replay);
        }
        IdempotencyDecision::Conflict => return Err(ApiError::IdempotencyConflict),
        IdempotencyDecision::InProgress => return Err(ApiError::IdempotencyInProgress),
        IdempotencyDecision::Reserved => {}
    }
    let pool = match state.database.put_node_pool(&name, &command, now).await {
        Ok(pool) => pool,
        Err(error) => {
            state
                .database
                .abandon_idempotency(&scope, key, &request_hash)
                .await?;
            return Err(error.into());
        }
    };
    state
        .database
        .record_audit(
            Some(actor.user_id),
            None,
            None,
            "node_pool.put",
            serde_json::json!({"name": pool.name, "enabled": pool.enabled}),
            now,
        )
        .await?;
    let body = serde_json::to_string(&pool)
        .map_err(|_| ApiError::BadRequest("response serialization failed"))?;
    state
        .database
        .finish_idempotency(&scope, key, &request_hash, StatusCode::OK.as_u16(), &body)
        .await?;
    json_response(StatusCode::OK, body)
}

#[utoipa::path(delete, path = "/api/v1/admin/node-pools/{name}", params(("name" = String, Path)), responses((status = 204), (status = 403, body = super::ErrorEnvelope), (status = 409, body = super::ErrorEnvelope)))]
pub(crate) async fn delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.may_manage_system() || actor.has_template_restriction() {
        return Err(ApiError::Forbidden);
    }
    state.database.delete_node_pool(&name).await?;
    state
        .database
        .record_audit(
            Some(actor.user_id),
            None,
            None,
            "node_pool.delete",
            serde_json::json!({"name": name}),
            unix_timestamp()?,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
