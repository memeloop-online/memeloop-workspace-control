use std::{collections::BTreeSet, sync::Arc};

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
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
    injections::{InjectionScope, validate_injection_key},
    storage::{IdempotencyCompletion, IdempotencyDecision, StorageError},
};

use super::{abandon, authorize, no_content_response, parse_scope};

const MAX_BATCH_KEYS: usize = 100;

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub(in crate::api) struct BatchDeleteRequest {
    pub keys: Vec<String>,
}

#[utoipa::path(
    post,
    path = "/api/v1/injections/{scope}/{scope_id}/batch-delete",
    params(
        ("scope" = String, Path),
        ("scope_id" = Uuid, Path),
        ("Idempotency-Key" = String, Header)
    ),
    request_body = BatchDeleteRequest,
    responses(
        (status = 204, description = "Injection batch deleted; missing keys are successful"),
        (status = 400, body = crate::api::ErrorEnvelope),
        (status = 401, body = crate::api::ErrorEnvelope),
        (status = 403, body = crate::api::ErrorEnvelope),
        (status = 409, body = crate::api::ErrorEnvelope)
    )
)]
pub(in crate::api) async fn batch_delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((scope, scope_id)): Path<(String, Uuid)>,
    Json(request): Json<BatchDeleteRequest>,
) -> Result<Response, ApiError> {
    let keys = normalized_keys(request.keys)?;
    let actor = principal(&state, &headers).await?;
    let scope_ref = parse_scope(&scope, scope_id)?;
    authorize(&state, &actor, scope_ref, true, false).await?;
    let allow_locked = scope_ref.scope == InjectionScope::Organization
        && actor.allows(Permission::ManageLockedInjections, scope_ref.scope_id);
    let idempotency_key = idempotency_key(&headers)?;
    let request_hash = hash(&(scope_ref, &keys))?;
    let idempotency_scope = format!(
        "{}:batch-delete-injections:{}:{}",
        actor.user_id,
        scope_ref.scope.as_str(),
        scope_ref.scope_id
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
        .delete_injections_and_enqueue_reconciles(
            scope_ref,
            &keys,
            allow_locked,
            actor.user_id,
            now,
            IdempotencyCompletion {
                scope: &idempotency_scope,
                key: idempotency_key,
                request_hash: &request_hash,
                status_code: StatusCode::NO_CONTENT.as_u16(),
                response_json: "",
            },
        )
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
    no_content_response()
}

fn normalized_keys(keys: Vec<String>) -> Result<Vec<String>, ApiError> {
    if keys.is_empty() || keys.len() > MAX_BATCH_KEYS {
        return Err(ApiError::BadRequest(
            "injection batch must contain between 1 and 100 keys",
        ));
    }
    for key in &keys {
        validate_injection_key(key)?;
    }
    let unique = keys.iter().collect::<BTreeSet<_>>();
    if unique.len() != keys.len() {
        return Err(ApiError::BadRequest("injection batch keys must be unique"));
    }
    let mut keys = keys;
    keys.sort_unstable();
    Ok(keys)
}
