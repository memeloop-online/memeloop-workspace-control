use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header::AUTHORIZATION},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{auth::Permission, storage::IssuedWebShellTicket, workspaces::WorkspaceState};

use super::{ApiError, AppState, auth::principal, idempotency::unix_timestamp};

const TICKET_TTL_SECONDS: i64 = 60;

#[derive(Serialize, ToSchema)]
pub(super) struct WebShellTicketResponse {
    #[serde(flatten)]
    pub issued: IssuedWebShellTicket,
    pub web_shell_url: String,
}

#[utoipa::path(
    post,
    path = "/api/v1/workspaces/{workspace_id}/web-shell-tickets",
    params(("workspace_id" = Uuid, Path)),
    responses(
        (status = 201, description = "One-time Web Shell ticket", body = WebShellTicketResponse),
        (status = 401, body = super::ErrorEnvelope),
        (status = 403, body = super::ErrorEnvelope),
        (status = 409, body = super::ErrorEnvelope)
    )
)]
pub(super) async fn issue(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(workspace_id): Path<Uuid>,
) -> Result<(StatusCode, Json<WebShellTicketResponse>), ApiError> {
    let actor = principal(&state, &headers).await?;
    let workspace = state.database.get_workspace(workspace_id).await?;
    if !actor.allows(Permission::ConnectWorkspace, workspace.organization_id) {
        return Err(ApiError::Forbidden);
    }
    if workspace.state != WorkspaceState::Ready {
        return Err(ApiError::WorkspaceNotConnectable);
    }
    let issued = state
        .database
        .issue_web_shell_ticket(
            workspace.organization_id,
            workspace.id,
            actor.user_id,
            unix_timestamp()?,
            TICKET_TTL_SECONDS,
        )
        .await?;
    let path = format!("/shell/{}/?ticket={}", workspace.short_id, issued.ticket);
    let web_shell_url = state
        .config
        .web_shell_public_origin
        .as_ref()
        .map_or_else(|| path.clone(), |origin| format!("{origin}{path}"));
    Ok((
        StatusCode::CREATED,
        Json(WebShellTicketResponse {
            issued,
            web_shell_url,
        }),
    ))
}

#[utoipa::path(
    get,
    path = "/api/v1/internal/web-shell/authorize",
    responses(
        (status = 200, description = "Higress may proxy this WebSocket handshake"),
        (status = 401, body = super::ErrorEnvelope)
    )
)]
pub(super) async fn authorize(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    verify_internal_caller(&state, &headers)?;
    let forwarded_uri = headers
        .get("x-forwarded-uri")
        .and_then(|value| value.to_str().ok());
    // ttyd serves the HTML and static assets over ordinary HTTP, then appends the
    // page query string to its `/ws` URL. Only the WebSocket upgrade carries
    // terminal traffic, so consume the one-time ticket at that boundary. If the
    // initial page request consumed it, ttyd's subsequent upgrade would fail.
    if !forwarded_uri.is_some_and(is_ttyd_websocket_uri) {
        return Ok(StatusCode::OK.into_response());
    }
    let workspace_id = match headers
        .get("x-mwc-workspace-id")
        .and_then(|value| value.to_str().ok())
    {
        Some(value) => value.parse::<Uuid>().map_err(|_| ApiError::Unauthorized)?,
        None => {
            let short_id = forwarded_uri
                .and_then(workspace_short_id_from_uri)
                .ok_or(ApiError::Unauthorized)?;
            state.database.get_workspace_by_short_id(short_id).await?.id
        }
    };
    let ticket = headers
        .get("x-mwc-web-shell-ticket")
        .and_then(|value| value.to_str().ok())
        .or_else(|| forwarded_uri.and_then(ticket_from_uri))
        .ok_or(ApiError::Unauthorized)?;
    let identity = state
        .database
        .consume_web_shell_ticket(ticket, workspace_id, unix_timestamp()?)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    let workspace = state.database.get_workspace(workspace_id).await?;
    if workspace.state != WorkspaceState::Ready
        || workspace.organization_id != identity.organization_id
    {
        return Err(ApiError::Unauthorized);
    }

    let mut response = StatusCode::OK.into_response();
    response.headers_mut().insert(
        "x-mwc-user-id",
        HeaderValue::from_str(&identity.user_id.to_string()).map_err(|_| ApiError::Unauthorized)?,
    );
    response.headers_mut().insert(
        "x-mwc-workspace-id",
        HeaderValue::from_str(&identity.workspace_id.to_string())
            .map_err(|_| ApiError::Unauthorized)?,
    );
    Ok(response)
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

fn ticket_from_uri(uri: &str) -> Option<&str> {
    uri.split_once('?')?.1.split('&').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (key == "ticket" && !value.is_empty()).then_some(value)
    })
}

fn workspace_short_id_from_uri(uri: &str) -> Option<&str> {
    let path = uri.split_once('?').map_or(uri, |(path, _)| path);
    let mut segments = path.trim_matches('/').split('/');
    (segments.next()? == "shell")
        .then(|| segments.next())
        .flatten()
        .filter(|short_id| short_id.len() == 8)
}

fn is_ttyd_websocket_uri(uri: &str) -> bool {
    uri.split_once('?')
        .map_or(uri, |(path, _)| path)
        .trim_end_matches('/')
        .ends_with("/ws")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_url_safe_ticket_without_decoding_secrets() {
        assert_eq!(
            ticket_from_uri("/shell/abc/?other=1&ticket=A_b-09&x=2"),
            Some("A_b-09")
        );
        assert_eq!(ticket_from_uri("/shell/abc/"), None);
        assert_eq!(
            workspace_short_id_from_uri("/shell/01abcdef/?ticket=one"),
            Some("01abcdef")
        );
        assert!(is_ttyd_websocket_uri("/shell/01abcdef/ws?ticket=A_b-09"));
        assert!(!is_ttyd_websocket_uri("/shell/01abcdef/?ticket=A_b-09"));
    }
}
