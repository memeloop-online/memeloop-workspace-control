use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Response,
};

use crate::storage::{CreateOrganization, IdempotencyDecision, Organization};

#[utoipa::path(
    get,
    path = "/api/v1/organizations",
    responses(
        (status = 200, description = "Visible organizations", body = [Organization]),
        (status = 401, body = super::ErrorEnvelope)
    )
)]
pub(super) async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<Organization>>, ApiError> {
    let actor = principal(&state, &headers).await?;
    Ok(Json(
        state
            .database
            .list_organizations_for(actor.user_id, actor.system_admin)
            .await?,
    ))
}

use super::{
    ApiError, AppState,
    auth::principal,
    idempotency::{
        IDEMPOTENCY_TTL_SECONDS, hash, idempotency_key, json_response, replay_response,
        unix_timestamp,
    },
};

#[utoipa::path(
    post,
    path = "/api/v1/organizations",
    request_body = CreateOrganization,
    params(("Idempotency-Key" = String, Header)),
    responses(
        (status = 201, description = "Organization created", body = Organization),
        (status = 401, body = super::ErrorEnvelope),
        (status = 403, body = super::ErrorEnvelope)
    )
)]
pub(super) async fn create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut command): Json<CreateOrganization>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.system_admin {
        return Err(ApiError::Forbidden);
    }
    command.owner_user_id = if command.owner_user_id.is_nil() {
        actor.user_id
    } else {
        command.owner_user_id
    };
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&command)?;
    let scope = format!("{}:create-organization", actor.user_id);
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
    let organization = match state.database.create_organization(command, now).await {
        Ok(organization) => organization,
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
            Some(organization.id),
            None,
            "organization.create",
            serde_json::json!({"name": organization.name}),
            now,
        )
        .await?;
    let response_json = serde_json::to_string(&organization)
        .map_err(|_| ApiError::BadRequest("response serialization failed"))?;
    state
        .database
        .finish_idempotency(
            &scope,
            key,
            &request_hash,
            StatusCode::CREATED.as_u16(),
            &response_json,
        )
        .await?;
    json_response(StatusCode::CREATED, response_json)
}
