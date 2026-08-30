use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::storage::CreatePluginUiSession;

use super::super::{
    ApiError, AppState,
    auth::principal,
    idempotency::{hash, idempotency_key, unix_timestamp},
};

const SESSION_TTL_SECONDS: i64 = 5 * 60;
mod bridge;
mod helpers;
pub(crate) use bridge::bridge;
use helpers::{
    cookie_name, cookie_value, digest, finish, random_token, reserve, validate_session_package,
};

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct PluginUiSessionView {
    launch_url: String,
    expires_at: i64,
    channel_nonce: String,
    allowed_bridge_methods: Vec<String>,
    bridge_url: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SurfaceAssetQuery {
    ticket: Option<String>,
}

pub(crate) async fn create_surface_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((plugin_id, surface_id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let actor = principal(&state, &headers).await?;
    state.plugins.synchronize().await?;
    let plugin = state
        .plugins
        .plugin_views()?
        .into_iter()
        .find(|item| item.manifest.id == plugin_id && item.enabled)
        .ok_or(ApiError::PluginDistribution("plugin_not_found"))?;
    if !plugin
        .approved_contributions
        .iter()
        .any(|value| value == "ui_surfaces")
    {
        return Err(ApiError::PluginDistribution(
            "plugin_capability_not_approved",
        ));
    }
    let surface = plugin
        .manifest
        .ui_surfaces
        .into_iter()
        .find(|item| item.id == surface_id)
        .ok_or(ApiError::PluginDistribution("plugin_surface_not_found"))?;
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&(&plugin_id, &surface_id))?;
    let scope = format!(
        "{}:plugin-ui-session:{plugin_id}:{surface_id}",
        actor.user_id
    );
    let now = unix_timestamp()?;
    if let Some(response) = reserve(&state, &scope, key, &request_hash, now).await? {
        return Ok(response);
    }
    let ticket = random_token()?;
    let cookie = random_token()?;
    let channel_nonce = random_token()?;
    let stored = state
        .database
        .create_plugin_ui_session(CreatePluginUiSession {
            plugin_id: &plugin_id,
            surface_id: &surface_id,
            user_id: actor.user_id,
            ticket_hash: &digest(&ticket),
            cookie_hash: &digest(&cookie),
            channel_nonce: &channel_nonce,
            allowed_bridge_methods: &surface.allowed_bridge_methods,
            entrypoint: &surface.entrypoint,
            package_digest: &plugin.package_digest,
            expires_at: now + SESSION_TTL_SECONDS,
            now,
        })
        .await?;
    let base = format!("/api/v1/plugin-ui/{plugin_id}/{}/", stored.id);
    let view = PluginUiSessionView {
        launch_url: format!("{base}{}?ticket={ticket}.{cookie}", surface.entrypoint),
        expires_at: stored.expires_at,
        channel_nonce,
        allowed_bridge_methods: stored.allowed_bridge_methods,
        bridge_url: format!("{base}bridge"),
    };
    finish(&state, &scope, key, &request_hash, &view).await
}

pub(crate) async fn surface_asset(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((plugin_id, session_id, asset_path)): Path<(String, Uuid, String)>,
    Query(query): Query<SurfaceAssetQuery>,
) -> Result<Response, ApiError> {
    let now = unix_timestamp()?;
    let (session, set_cookie) = if let Some(ticket_and_cookie) = query.ticket {
        let (ticket, cookie) = ticket_and_cookie
            .split_once('.')
            .ok_or(ApiError::PluginDistribution("plugin_ui_session_invalid"))?;
        let session = state
            .database
            .consume_plugin_ui_ticket(&digest(ticket), now)
            .await?;
        if session.id != session_id
            || session.plugin_id != plugin_id
            || session.entrypoint != asset_path
        {
            return Err(ApiError::PluginDistribution("plugin_ui_session_invalid"));
        }
        let cookie_header = format!(
            "{}={}; Path=/api/v1/plugin-ui/{plugin_id}/{session_id}/; Max-Age={SESSION_TTL_SECONDS}; HttpOnly; Secure; SameSite=Strict",
            cookie_name(session_id),
            cookie
        );
        (session, Some(cookie_header))
    } else {
        let cookie = cookie_value(&headers, session_id)
            .ok_or(ApiError::PluginDistribution("plugin_ui_session_invalid"))?;
        (
            state
                .database
                .plugin_ui_session_by_cookie(session_id, &digest(cookie), now)
                .await?,
            None,
        )
    };
    validate_session_package(&state, &session, &plugin_id).await?;
    let asset = state
        .database
        .plugin_asset(&plugin_id, &asset_path)
        .await?
        .ok_or(ApiError::PluginDistribution("plugin_asset_not_found"))?;
    let mut response = Response::new(asset.content.into());
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&asset.media_type)
            .map_err(|_| ApiError::PluginDistribution("plugin_package_invalid"))?,
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert("content-security-policy", HeaderValue::from_static("default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'none'; frame-ancestors 'self'; base-uri 'none'; form-action 'none'"));
    if let Some(cookie) = set_cookie {
        headers.insert(
            header::SET_COOKIE,
            HeaderValue::from_str(&cookie)
                .map_err(|_| ApiError::PluginDistribution("plugin_ui_session_invalid"))?,
        );
    }
    Ok(response)
}
