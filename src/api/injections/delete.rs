use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use uuid::Uuid;

use crate::{
    api::{
        ApiError, AppState,
        auth::principal,
        idempotency::{
            IDEMPOTENCY_TTL_SECONDS, hash, idempotency_key, replay_response, unix_timestamp,
        },
    },
    auth::Permission,
    injections::InjectionScope,
    storage::{IdempotencyDecision, StorageError},
};

use super::{authorize, parse_scope};

#[utoipa::path(
    delete,
    path = "/api/v1/injections/{scope}/{scope_id}/{key}",
    params(
        ("scope" = String, Path),
        ("scope_id" = Uuid, Path),
        ("key" = String, Path),
        ("Idempotency-Key" = String, Header)
    ),
    responses(
        (status = 204, description = "Injection deleted; deleting a missing key is successful"),
        (status = 401, body = crate::api::ErrorEnvelope),
        (status = 403, body = crate::api::ErrorEnvelope),
        (status = 409, body = crate::api::ErrorEnvelope)
    )
)]
pub(in crate::api) async fn delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((scope, scope_id, key)): Path<(String, Uuid, String)>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    let scope_ref = parse_scope(&scope, scope_id)?;
    authorize(&state, &actor, scope_ref, true, false).await?;
    let allow_locked = scope_ref.scope == InjectionScope::Organization
        && actor.allows(Permission::ManageLockedInjections, scope_ref.scope_id);
    let idempotency_key = idempotency_key(&headers)?;
    let request_hash = hash(&(scope_ref, &key))?;
    let idempotency_scope = format!(
        "{}:delete-injection:{}:{}:{}",
        actor.user_id,
        scope_ref.scope.as_str(),
        scope_ref.scope_id,
        key
    );
    let now = unix_timestamp()?;
    match state
        .database
        .begin_idempotency(
            &idempotency_scope,
            idempotency_key,
            &request_hash,
            now,
            now + IDEMPOTENCY_TTL_SECONDS,
        )
        .await?
    {
        IdempotencyDecision::Replay(replay) => return replay_response(replay),
        IdempotencyDecision::Conflict => return Err(ApiError::IdempotencyConflict),
        IdempotencyDecision::InProgress => return Err(ApiError::IdempotencyInProgress),
        IdempotencyDecision::Reserved => {}
    }
    match state
        .database
        .delete_injection(scope_ref, &key, allow_locked, actor.user_id, now)
        .await
    {
        Ok(_) => {}
        Err(StorageError::InvalidInjectionLock) => {
            abandon(&state, &idempotency_scope, idempotency_key, &request_hash).await?;
            return Err(ApiError::Forbidden);
        }
        Err(error) => {
            abandon(&state, &idempotency_scope, idempotency_key, &request_hash).await?;
            return Err(error.into());
        }
    }
    if let Err(error) = state
        .database
        .enqueue_injection_reconciles(scope_ref, now)
        .await
    {
        abandon(&state, &idempotency_scope, idempotency_key, &request_hash).await?;
        return Err(error.into());
    }
    finish_empty(&state, &idempotency_scope, idempotency_key, &request_hash).await
}

async fn abandon(
    state: &AppState,
    scope: &str,
    key: &str,
    request_hash: &str,
) -> Result<(), ApiError> {
    state
        .database
        .abandon_idempotency(scope, key, request_hash)
        .await?;
    Ok(())
}

async fn finish_empty(
    state: &AppState,
    scope: &str,
    key: &str,
    request_hash: &str,
) -> Result<Response, ApiError> {
    state
        .database
        .finish_idempotency(
            scope,
            key,
            request_hash,
            StatusCode::NO_CONTENT.as_u16(),
            "",
        )
        .await?;
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(axum::body::Body::empty())
        .map_err(|_| ApiError::BadRequest("response could not be built"))
}
