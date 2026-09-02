//! Higress external-auth and browser bootstrap endpoints for HTTP mappings.

use std::sync::Arc;

use axum::{
    extract::State,
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{AUTHORIZATION, LOCATION, SET_COOKIE},
    },
    response::{IntoResponse, Response},
};
use base64::Engine as _;
use uuid::Uuid;

use crate::storage::hash_secret;

use super::{ApiError, AppState};

const SESSION_TTL_SECONDS: i64 = 8 * 60 * 60;
const COOKIE: &str = "__Host-mwc-port-session";

/// Called only by Higress's fail-closed external-auth plugin. The plugin must
/// forward the original Host and URI but never client supplied identity headers.
pub(in crate::api) async fn authorize(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    verify_internal_caller(&state, &headers)?;
    let mapping_id = mapping_id_from_host(&headers, &state)?;
    let mapping = state
        .database
        .get_port_mapping(mapping_id)
        .await
        .map_err(|_| ApiError::Unauthorized)?;
    let workspace = state
        .database
        .get_workspace(mapping.workspace_id)
        .await
        .map_err(|_| ApiError::Unauthorized)?;
    if workspace.state != crate::workspaces::WorkspaceState::Ready {
        return Err(ApiError::Unauthorized);
    }
    let now = super::unix_timestamp()?;
    if let Some(cookie) = cookie(&headers, COOKIE)
        && state
            .database
            .port_mapping_session_valid(mapping_id, &hash_secret(cookie), now)
            .await?
    {
        return Ok(StatusCode::OK.into_response());
    }
    if let Some(ticket) = forwarded_uri(&headers)
        .filter(|uri| uri.starts_with("/_mwc/bootstrap?"))
        .and_then(ticket_from_uri)
    {
        return exchange_ticket(&state, mapping_id, ticket, now).await;
    }
    Err(ApiError::Unauthorized)
}

/// The external-auth response itself performs the one-time browser bootstrap.
/// Returning the redirect here avoids a second Higress upstream hop while
/// ensuring the workspace process never receives or logs the ticket.
async fn exchange_ticket(
    state: &AppState,
    mapping_id: Uuid,
    ticket: &str,
    now: i64,
) -> Result<Response, ApiError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| ApiError::Unauthorized)?;
    let session = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let expires_at = now
        .checked_add(SESSION_TTL_SECONDS)
        .ok_or(ApiError::Unauthorized)?;
    if state
        .database
        .exchange_port_mapping_ticket(mapping_id, ticket, &hash_secret(&session), now, expires_at)
        .await?
        .is_none()
    {
        return Err(ApiError::Unauthorized);
    }
    let mut response = StatusCode::SEE_OTHER.into_response();
    let cookie = format!(
        "{COOKIE}={session}; Path=/; Max-Age={SESSION_TTL_SECONDS}; HttpOnly; Secure; SameSite=Lax"
    );
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|_| ApiError::Unauthorized)?,
    );
    response
        .headers_mut()
        .insert(LOCATION, HeaderValue::from_static("/"));
    Ok(response)
}

fn mapping_id_from_host(headers: &HeaderMap, state: &AppState) -> Result<Uuid, ApiError> {
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get("host"))
        .and_then(|v| v.to_str().ok())
        .ok_or(ApiError::Unauthorized)?;
    let suffix = state
        .config
        .port_mapping_public_domain
        .as_deref()
        .ok_or(ApiError::Unauthorized)?;
    let normalized = host
        .trim_end_matches('.')
        .split_once(':')
        .map_or(host, |(hostname, _)| hostname)
        .to_ascii_lowercase();
    let value = normalized
        .trim_end_matches('.')
        .strip_suffix(&format!(".{suffix}"))
        .and_then(|v| v.strip_prefix("p-"))
        .ok_or(ApiError::Unauthorized)?;
    Uuid::parse_str(value).map_err(|_| ApiError::Unauthorized)
}

fn verify_internal_caller(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if !state.web_shell_internal_caller_allowed(token) {
        return Err(ApiError::Unauthorized);
    }
    Ok(())
}

fn forwarded_uri(headers: &HeaderMap) -> Option<&str> {
    headers.get("x-forwarded-uri").and_then(|v| v.to_str().ok())
}

fn ticket_from_uri(uri: &str) -> Option<&str> {
    uri.split_once('?')?.1.split('&').find_map(|v| {
        let (k, v) = v.split_once('=')?;
        (k == "ticket" && !v.is_empty()).then_some(v)
    })
}

fn cookie<'a>(headers: &'a HeaderMap, wanted: &str) -> Option<&'a str> {
    headers
        .get("cookie")?
        .to_str()
        .ok()?
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix(&format!("{wanted}=")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_and_cookie_parsing_do_not_accept_near_matches() {
        assert_eq!(
            ticket_from_uri("/?other=one&ticket=valid_ticket&next=two"),
            Some("valid_ticket")
        );
        assert_eq!(ticket_from_uri("/?tickets=wrong"), None);
        let mut headers = HeaderMap::new();
        headers.insert(
            "cookie",
            HeaderValue::from_static("other=x; __Host-mwc-port-session=secret; x=y"),
        );
        assert_eq!(cookie(&headers, COOKIE), Some("secret"));
        assert_eq!(cookie(&headers, "session"), None);
    }
}
