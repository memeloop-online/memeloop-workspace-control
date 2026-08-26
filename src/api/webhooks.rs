use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use serde::Deserialize;
use utoipa::IntoParams;
use uuid::Uuid;

use crate::{
    auth::Permission,
    storage::{CreateWebhookSubscription, IdempotencyDecision, WebhookSubscriptionSummary},
};

use super::{
    ApiError, AppState,
    auth::principal,
    idempotency::{
        IDEMPOTENCY_TTL_SECONDS, hash, idempotency_key, json_response, replay_response,
        unix_timestamp,
    },
};

#[derive(Debug, Deserialize, IntoParams)]
pub(super) struct WebhookQuery {
    organization_id: Uuid,
}

#[utoipa::path(get, path = "/api/v1/webhooks", params(WebhookQuery), responses((status = 200, body = [WebhookSubscriptionSummary]), (status = 403, body = super::ErrorEnvelope)))]
pub(super) async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<WebhookQuery>,
) -> Result<Json<Vec<WebhookSubscriptionSummary>>, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.allows(Permission::ManageOrganization, query.organization_id) {
        return Err(ApiError::Forbidden);
    }
    Ok(Json(
        state
            .database
            .list_webhook_subscriptions(query.organization_id)
            .await?,
    ))
}

#[utoipa::path(post, path = "/api/v1/webhooks", request_body = CreateWebhookSubscription, params(("Idempotency-Key" = String, Header)), responses((status = 201, body = WebhookSubscriptionSummary), (status = 403, body = super::ErrorEnvelope)))]
pub(super) async fn create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(command): Json<CreateWebhookSubscription>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.allows(Permission::ManageOrganization, command.organization_id) {
        return Err(ApiError::Forbidden);
    }
    let cipher = state
        .cipher
        .as_ref()
        .ok_or(ApiError::EncryptionUnavailable)?;
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&command)?;
    let scope = format!("{}:create-webhook", actor.user_id);
    let now = unix_timestamp()?;
    match state
        .database
        .begin_idempotency(
            &scope,
            key,
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
    let summary = match state
        .database
        .create_webhook_subscription(cipher, command, actor.user_id, now)
        .await
    {
        Ok(summary) => summary,
        Err(error) => {
            state
                .database
                .abandon_idempotency(&scope, key, &request_hash)
                .await?;
            return Err(error.into());
        }
    };
    state.database.record_audit(Some(actor.user_id), Some(summary.organization_id), None, "webhook.create", serde_json::json!({"webhook_id": summary.id, "url": summary.url, "event_prefix": summary.event_prefix}), now).await?;
    let body = serde_json::to_string(&summary)
        .map_err(|_| ApiError::BadRequest("response serialization failed"))?;
    state
        .database
        .finish_idempotency(
            &scope,
            key,
            &request_hash,
            StatusCode::CREATED.as_u16(),
            &body,
        )
        .await?;
    json_response(StatusCode::CREATED, body)
}
