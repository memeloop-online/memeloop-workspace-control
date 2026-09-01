use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};

use crate::storage::{CreateOrganization, IdempotencyDecision, Organization, OrganizationPage};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

#[derive(Debug, Default, Deserialize, IntoParams)]
pub(super) struct OrganizationListQuery {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    pub search: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub(super) struct UpdateOrganizationRequest {
    pub name: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/organizations",
    params(OrganizationListQuery),
    responses(
        (status = 200, description = "Visible organizations", body = OrganizationPage),
        (status = 401, body = super::ErrorEnvelope)
    )
)]
pub(super) async fn list_page(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<OrganizationListQuery>,
) -> Result<Json<OrganizationPage>, ApiError> {
    let actor = principal(&state, &headers).await?;
    Ok(Json(
        state
            .database
            .list_organizations_page_for(
                actor.user_id,
                actor.may_manage_system(),
                query.limit,
                query.cursor.as_deref(),
                query.search.as_deref(),
            )
            .await?,
    ))
}

#[utoipa::path(
    put,
    path = "/api/v1/organizations/{organization_id}",
    request_body = UpdateOrganizationRequest,
    params(("organization_id" = Uuid, Path)),
    responses((status = 200, body = Organization), (status = 403, body = super::ErrorEnvelope))
)]
pub(super) async fn update(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(organization_id): Path<Uuid>,
    Json(request): Json<UpdateOrganizationRequest>,
) -> Result<Json<Organization>, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.allows(crate::auth::Permission::ManageOrganization, organization_id) {
        return Err(ApiError::Forbidden);
    }
    let organization = state
        .database
        .rename_organization(organization_id, &request.name)
        .await?;
    state
        .database
        .record_audit(
            Some(actor.user_id),
            Some(organization_id),
            None,
            "organization.update",
            serde_json::json!({"name": organization.name}),
            unix_timestamp()?,
        )
        .await?;
    Ok(Json(organization))
}

#[utoipa::path(
    delete,
    path = "/api/v1/organizations/{organization_id}",
    params(("organization_id" = Uuid, Path)),
    responses((status = 204), (status = 403, body = super::ErrorEnvelope), (status = 409, body = super::ErrorEnvelope))
)]
pub(super) async fn delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(organization_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.may_manage_system() {
        return Err(ApiError::Forbidden);
    }
    state
        .database
        .delete_organization_if_empty(organization_id)
        .await?;
    state
        .database
        .record_audit(
            Some(actor.user_id),
            Some(organization_id),
            None,
            "organization.delete",
            serde_json::json!({}),
            unix_timestamp()?,
        )
        .await?;
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(axum::body::Body::empty())
        .map_err(|_| ApiError::BadRequest("response could not be built"))
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
    if !actor.may_manage_system() {
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
