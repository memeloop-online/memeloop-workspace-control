use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::storage::{ApiKeySummary, StoredUserProfile};

use crate::api::{ApiError, AppState, auth::principal, idempotency::unix_timestamp};

const GENERATED_AVATAR_PREFIX: &str = "data:image/svg+xml;base64,";

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(in crate::api) struct UserProfileResponse {
    pub display_name: String,
    pub avatar_url: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub(in crate::api) struct UpdateUserProfileRequest {
    pub display_name: String,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub(in crate::api) struct CreateApiKeyRequest {
    pub name: String,
}

#[derive(Serialize, ToSchema)]
pub(in crate::api) struct CreatedApiKeyResponse {
    #[serde(flatten)]
    pub summary: ApiKeySummary,
    /// One-time plaintext API key. It is never returned by list operations or stored as plaintext;
    /// if it is lost, revoke this key and create a replacement.
    pub token: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/me/profile",
    responses(
        (status = 200, body = UserProfileResponse),
        (status = 401, body = crate::api::ErrorEnvelope)
    )
)]
pub(in crate::api) async fn get_profile(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<UserProfileResponse>, ApiError> {
    let actor = principal(&state, &headers).await?;
    let profile = state.database.get_user_profile(actor.user_id).await?;
    Ok(Json(profile_response(profile)))
}

#[utoipa::path(
    put,
    path = "/api/v1/me/profile",
    request_body = UpdateUserProfileRequest,
    responses(
        (status = 200, body = UserProfileResponse),
        (status = 400, body = crate::api::ErrorEnvelope),
        (status = 401, body = crate::api::ErrorEnvelope)
    )
)]
pub(in crate::api) async fn update_profile(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<UpdateUserProfileRequest>,
) -> Result<Json<UserProfileResponse>, ApiError> {
    let actor = principal(&state, &headers).await?;
    let avatar_url = request
        .avatar_url
        .as_deref()
        .and_then(|value| (!value.starts_with(GENERATED_AVATAR_PREFIX)).then_some(value));
    let profile = state
        .database
        .update_user_profile(actor.user_id, &request.display_name, avatar_url)
        .await?;
    state
        .database
        .record_audit(
            Some(actor.user_id),
            None,
            None,
            "user.profile.update",
            serde_json::json!({"fields": ["display_name", "avatar_url"]}),
            unix_timestamp()?,
        )
        .await?;
    Ok(Json(profile_response(profile)))
}

#[utoipa::path(
    get,
    path = "/api/v1/me/api-keys",
    responses(
        (status = 200, body = [ApiKeySummary]),
        (status = 401, body = crate::api::ErrorEnvelope)
    )
)]
pub(in crate::api) async fn list_api_keys(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<ApiKeySummary>>, ApiError> {
    let actor = principal(&state, &headers).await?;
    Ok(Json(state.database.list_api_keys(actor.user_id).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/me/api-keys",
    request_body = CreateApiKeyRequest,
    description = "Creates an API key for the authenticated user. The plaintext token is shown only in this response. If the response or token is lost, revoke the key and create a replacement; this endpoint does not support Idempotency-Key replay because plaintext keys are not retained.",
    responses(
        (status = 201, description = "Created; token is shown exactly once", body = CreatedApiKeyResponse),
        (status = 400, body = crate::api::ErrorEnvelope),
        (status = 401, body = crate::api::ErrorEnvelope),
        (status = 409, body = crate::api::ErrorEnvelope)
    )
)]
pub(in crate::api) async fn create_api_key(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<CreatedApiKeyResponse>), ApiError> {
    let actor = principal(&state, &headers).await?;
    let created = state
        .database
        .create_api_key(actor.user_id, &request.name, unix_timestamp()?)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedApiKeyResponse {
            summary: created.summary,
            token: created.token,
        }),
    ))
}

#[utoipa::path(
    delete,
    path = "/api/v1/me/api-keys/{key_id}",
    params(("key_id" = Uuid, Path)),
    responses(
        (status = 204),
        (status = 401, body = crate::api::ErrorEnvelope),
        (status = 404, body = crate::api::ErrorEnvelope),
        (status = 409, body = crate::api::ErrorEnvelope)
    )
)]
pub(in crate::api) async fn delete_api_key(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(key_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    state
        .database
        .revoke_api_key(actor.user_id, key_id, unix_timestamp()?)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

fn profile_response(profile: StoredUserProfile) -> UserProfileResponse {
    UserProfileResponse {
        display_name: profile.display_name,
        avatar_url: profile
            .avatar_url
            .unwrap_or_else(|| generated_avatar(profile.user_id)),
    }
}

fn generated_avatar(user_id: Uuid) -> String {
    let digest = Sha256::digest(user_id.as_bytes());
    let foreground = format!("#{:02x}{:02x}{:02x}", digest[0], digest[1], digest[2]);
    let background = format!(
        "#{:02x}{:02x}{:02x}",
        digest[3] | 0x80,
        digest[4] | 0x80,
        digest[5] | 0x80
    );
    let mut cells = String::new();
    for row in 0..5 {
        for column in 0..3 {
            if digest[6 + row] & (1 << column) == 0 {
                continue;
            }
            for x in [column, 4 - column] {
                cells.push_str(&format!(
                    "<rect x='{}' y='{}' width='16' height='16' rx='3'/>",
                    10 + x * 20,
                    10 + row * 20
                ));
                if column == 2 {
                    break;
                }
            }
        }
    }
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 120 120'><rect width='120' height='120' rx='24' fill='{background}'/><g fill='{foreground}'>{cells}</g></svg>"
    );
    format!("{GENERATED_AVATAR_PREFIX}{}", STANDARD.encode(svg))
}
