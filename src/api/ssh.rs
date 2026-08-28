use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Query, State},
    http::{HeaderMap, Response, StatusCode, header::CONTENT_TYPE},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use utoipa::IntoParams;

use crate::{
    injections::{InjectionKind, InjectionScope, InjectionValue},
    storage::InjectionScopeRef,
    workspaces::{AccessMode, WorkspaceState},
};

use super::{ApiError, AppState};

#[derive(Debug, Deserialize, IntoParams)]
pub(super) struct AuthorizedKeyQuery {
    /// OpenSSH login name in the form access+<workspace-short-id>.
    login: String,
    /// OpenSSH `%t` expansion.
    key_type: String,
    /// OpenSSH `%k` expansion.
    key_base64: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/internal/ssh/authorized-key",
    params(AuthorizedKeyQuery),
    responses(
        (status = 200, description = "A restricted authorized_keys line", body = String),
        (status = 401, body = super::ErrorEnvelope)
    )
)]
pub(super) async fn authorized_key(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AuthorizedKeyQuery>,
) -> Result<Response<Body>, ApiError> {
    verify_internal_caller(&state, &headers)?;
    validate_offered_key(&query)?;
    let short_id = query
        .login
        .strip_prefix("access+")
        .filter(|value| !value.is_empty())
        .ok_or(ApiError::Unauthorized)?;
    let workspace = state.database.get_workspace_by_short_id(short_id).await?;
    if workspace.state != WorkspaceState::Ready
        || workspace.template.access_mode != AccessMode::Public
    {
        return Err(ApiError::Unauthorized);
    }
    let cipher = state
        .cipher
        .as_ref()
        .ok_or(ApiError::EncryptionUnavailable)?;
    let mut authorized_user = None;
    for candidate in state
        .database
        .ssh_access_candidates(workspace.organization_id)
        .await?
    {
        let keys = state
            .database
            .load_injections(
                cipher,
                InjectionScopeRef {
                    scope: InjectionScope::User,
                    scope_id: candidate.user_id,
                },
            )
            .await?;
        if keys.iter().any(|item| public_key_matches(item, &query)) {
            authorized_user = Some(candidate.user_id);
            break;
        }
    }
    let user_id = authorized_user.ok_or(ApiError::Unauthorized)?;
    let namespace = state
        .config
        .installation_id
        .workspace_namespace(&workspace.short_id)
        .map_err(|_| ApiError::Unauthorized)?;
    let target = format!("workspace.{namespace}.svc.cluster.local:2222");
    let line = format!(
        "restrict,port-forwarding,permitopen=\"{target}\" {} {} mwc-user-{user_id}\n",
        query.key_type, query.key_base64
    );
    let response = Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .header("x-mwc-user-id", user_id.to_string())
        .header("x-mwc-workspace-id", workspace.id.to_string())
        .header("cache-control", "no-store")
        .body(Body::from(line))
        .map_err(|_| ApiError::Unauthorized)?;
    Ok(response)
}

#[utoipa::path(
    get,
    path = "/api/v1/internal/ssh/login-users",
    responses(
        (status = 200, description = "Derived OpenSSH login names", body = String),
        (status = 401, body = super::ErrorEnvelope)
    )
)]
pub(super) async fn login_users(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response<Body>, ApiError> {
    verify_internal_caller(&state, &headers)?;
    let mut body = state.database.list_public_ssh_logins().await?.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .header("cache-control", "no-store")
        .body(Body::from(body))
        .map_err(|_| ApiError::Unauthorized)
}

fn verify_internal_caller(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if !state.internal_caller_allowed(token) {
        return Err(ApiError::Unauthorized);
    }
    Ok(())
}

fn validate_offered_key(query: &AuthorizedKeyQuery) -> Result<(), ApiError> {
    let key_type_valid = !query.key_type.is_empty()
        && query.key_type.len() <= 128
        && query
            .key_type
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "@._-".contains(character));
    let key_blob_valid =
        query.key_base64.len() <= 16_384 && STANDARD.decode(query.key_base64.as_bytes()).is_ok();
    if !key_type_valid || !key_blob_valid {
        return Err(ApiError::Unauthorized);
    }
    Ok(())
}

fn public_key_matches(
    item: &crate::injections::InjectionItem,
    offered: &AuthorizedKeyQuery,
) -> bool {
    if item.kind != InjectionKind::SshPublicKey {
        return false;
    }
    let InjectionValue::Utf8(value) = &item.value else {
        return false;
    };
    let mut fields = value.split_ascii_whitespace();
    fields.next() == Some(offered.key_type.as_str())
        && fields.next() == Some(offered.key_base64.as_str())
}
