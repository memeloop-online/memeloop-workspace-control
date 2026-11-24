use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

use super::{ApiError, AppState, bearer};
use crate::{
    api::idempotency::unix_timestamp,
    storage::{OidcBindingResult, Principal, hash_secret},
};

use super::oidc_provider::ProviderError;

const FLOW_COOKIE: &str = "__Host-mwc-oidc";
const SESSION_COOKIE: &str = "__Host-mwc-session";
const FLOW_TTL: i64 = 300;
const SESSION_TTL: i64 = 4 * 60 * 60;

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(in crate::api) enum Action {
    Login,
    Bind,
    Unbind,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
enum Flow {
    Authorize {
        action: Action,
        nonce: String,
        verifier: String,
        started_at: i64,
        issuer: String,
    },
    Confirm {
        action: Action,
        issuer: String,
        subject: String,
    },
}

#[derive(Deserialize)]
pub(in crate::api) struct Start {
    action: Action,
}

#[derive(Deserialize)]
pub(in crate::api) struct Callback {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
}

pub(in crate::api) async fn csrf(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let oidc_route = request.uri().path().starts_with("/api/v1/auth/oidc/");
    if let Some(provider) = &state.oidc {
        let headers = request.headers();
        let read = matches!(
            *request.method(),
            Method::GET | Method::HEAD | Method::OPTIONS
        );
        if !read
            && (oidc_route
                || (!headers.contains_key(header::AUTHORIZATION)
                    && cookie(headers, SESSION_COOKIE).is_some()))
            && headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) != Some(provider.origin())
        {
            return ApiError::Forbidden.into_response();
        }
    }
    let mut response = next.run(request).await;
    if oidc_route {
        let headers = response.headers_mut();
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        headers.insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        );
    }
    response
}

pub(super) async fn principal(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<Principal>, ApiError> {
    let (Some(provider), Some(secret)) = (&state.oidc, cookie(headers, SESSION_COOKIE)) else {
        return Ok(None);
    };
    Ok(state
        .database
        .oidc_session_principal(&hash_secret(secret), provider.issuer(), unix_timestamp()?)
        .await?)
}

pub(in crate::api) async fn status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(serde_json::json!({
        "enabled": state.oidc.is_some(),
        "signed_in": principal(&state, &headers).await?.is_some(),
    })))
}

pub(in crate::api) async fn start(
    State(state): State<Arc<AppState>>,
    Json(Start { action }): Json<Start>,
) -> Result<Response, ApiError> {
    let provider = state.oidc.as_deref().ok_or(ApiError::Forbidden)?;
    let public_state = random_secret()?;
    let nonce = random_secret()?;
    let verifier = random_secret()?;
    let flow_cookie = random_secret()?;
    let authorization_url = provider
        .authorization_url(
            &public_state,
            &nonce,
            &verifier,
            !matches!(action, Action::Login),
        )
        .await
        .map_err(|_| ApiError::IdentityProviderUnavailable)?;
    let now = unix_timestamp()?;
    save_flow(
        &state,
        &authorize_key(&flow_cookie, &public_state),
        &Flow::Authorize {
            action,
            nonce,
            verifier,
            started_at: now,
            issuer: provider.issuer().to_owned(),
        },
        now,
    )
    .await?;
    Ok(with_cookie(
        Json(serde_json::json!({"authorization_url": authorization_url.as_str()})).into_response(),
        FLOW_COOKIE,
        &flow_cookie,
        FLOW_TTL,
    ))
}

pub(in crate::api) async fn callback(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    query: Result<Query<Callback>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, ApiError> {
    let Some(provider) = state.oidc.as_deref() else {
        return Err(ApiError::Forbidden);
    };
    let now = unix_timestamp()?;
    let (Some(original_cookie), Ok(Query(callback))) = (cookie(&headers, FLOW_COOKIE), query)
    else {
        return Ok(finish("/?oidc=failed"));
    };
    let Some(public_state) = callback
        .state
        .as_deref()
        .filter(|value| valid_secret(value))
    else {
        return Ok(finish("/?oidc=failed"));
    };
    let Some(Flow::Authorize {
        action,
        nonce,
        verifier,
        started_at,
        issuer,
    }) = take_flow(&state, &authorize_key(original_cookie, public_state), now).await?
    else {
        return Ok(finish("/?oidc=failed"));
    };
    let Some(code) = callback
        .code
        .filter(|_| callback.error.is_none() && issuer == provider.issuer())
    else {
        return Ok(finish("/?oidc=failed"));
    };
    let fresh_after = (!matches!(action, Action::Login)).then_some(started_at);
    let subject = match provider
        .verified_subject(&code, &nonce, &verifier, fresh_after)
        .await
    {
        Ok(subject) => subject,
        Err(error) => {
            if matches!(error, ProviderError::Unavailable) {
                tracing::warn!("OIDC provider unavailable during callback");
            }
            return Ok(finish("/?oidc=failed"));
        }
    };
    match action {
        Action::Login => {
            let secret = random_secret()?;
            let active = state
                .database
                .oidc_create_session(
                    provider.issuer(),
                    &subject,
                    &hash_secret(&secret),
                    now + SESSION_TTL,
                )
                .await?;
            if active {
                Ok(with_cookie(
                    finish("/"),
                    SESSION_COOKIE,
                    &secret,
                    SESSION_TTL,
                ))
            } else {
                Ok(finish("/?oidc=unbound"))
            }
        }
        Action::Bind | Action::Unbind => {
            let secret = random_secret()?;
            save_flow(
                &state,
                &hash_secret(&secret),
                &Flow::Confirm {
                    action,
                    issuer,
                    subject,
                },
                now,
            )
            .await?;
            let location = if matches!(action, Action::Bind) {
                "/?oidc=bind"
            } else {
                "/?oidc=unbind"
            };
            Ok(with_cookie(
                Redirect::to(location).into_response(),
                FLOW_COOKIE,
                &secret,
                FLOW_TTL,
            ))
        }
    }
}

pub(in crate::api) async fn confirm(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(Start { action: submitted }): Json<Start>,
) -> Result<Response, ApiError> {
    let provider = state.oidc.as_deref().ok_or(ApiError::Forbidden)?;
    let secret = cookie(&headers, FLOW_COOKIE).ok_or(ApiError::Forbidden)?;
    let token = bearer(&headers)?.ok_or(ApiError::Unauthorized)?;
    if state.database.oidc_owner(token).await?.is_none() {
        return Err(ApiError::Forbidden);
    }
    let now = unix_timestamp()?;
    let Some(Flow::Confirm {
        action,
        issuer,
        subject,
    }) = take_flow(&state, &hash_secret(secret), now).await?
    else {
        return Err(ApiError::ExpiredIdentityFlow);
    };
    if issuer != provider.issuer() || action != submitted {
        return Err(ApiError::Forbidden);
    }
    let result = match action {
        Action::Bind => {
            state
                .database
                .oidc_bind(token, &issuer, &subject, now)
                .await?
        }
        Action::Unbind => {
            state
                .database
                .oidc_unbind(token, &issuer, &subject, now)
                .await?
        }
        Action::Login => return Err(ApiError::Forbidden),
    };
    match result {
        OidcBindingResult::Changed => Ok(with_cookie(
            StatusCode::NO_CONTENT.into_response(),
            FLOW_COOKIE,
            "",
            0,
        )),
        OidcBindingResult::Unauthorized => Err(ApiError::Forbidden),
        OidcBindingResult::Conflict => Err(ApiError::IdentityAlreadyLinked),
    }
}

pub(in crate::api) async fn logout(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if state.oidc.is_none() {
        return Err(ApiError::Forbidden);
    }
    if let Some(secret) = cookie(&headers, SESSION_COOKIE) {
        state
            .database
            .oidc_revoke_session(&hash_secret(secret))
            .await?;
    }
    Ok(with_cookie(
        StatusCode::NO_CONTENT.into_response(),
        SESSION_COOKIE,
        "",
        0,
    ))
}

async fn save_flow(state: &AppState, key: &str, flow: &Flow, now: i64) -> Result<(), ApiError> {
    let payload = serde_json::to_string(flow).map_err(crate::storage::StorageError::from)?;
    Ok(state
        .database
        .oidc_put_flow(key, &payload, now + FLOW_TTL, now)
        .await?)
}

async fn take_flow(state: &AppState, key: &str, now: i64) -> Result<Option<Flow>, ApiError> {
    state
        .database
        .oidc_take_flow(key, now)
        .await?
        .map(|payload| serde_json::from_str(&payload).map_err(crate::storage::StorageError::from))
        .transpose()
        .map_err(ApiError::from)
}

fn authorize_key(secret: &str, state: &str) -> String {
    hash_secret(&format!("{secret}.{state}"))
}

fn valid_secret(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut cookies = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|entry| entry.trim().split_once('='))
        .filter_map(|(key, value)| (key == name).then_some(value));
    let value = cookies.next()?;
    (cookies.next().is_none() && valid_secret(value)).then_some(value)
}

fn random_secret() -> Result<String, ApiError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| ApiError::IdentityProviderUnavailable)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn with_cookie(mut response: Response, name: &str, value: &str, ttl: i64) -> Response {
    let cookie = format!("{name}={value}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age={ttl}");
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).expect("fixed cookie attributes and URL-safe token"),
    );
    response
}

fn finish(location: &str) -> Response {
    with_cookie(Redirect::to(location).into_response(), FLOW_COOKIE, "", 0)
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    use super::*;
    use crate::{
        api::{router, tests::test_state},
        auth::ApiKeyScope,
    };

    const ORIGIN: &str = "https://mwc.example";
    const ISSUER: &str = "https://idp.example/application/o/mwc/";
    const KEY: &str = "oidc-test-key-valid-12345678901234567890";

    #[tokio::test]
    async fn browser_proof_needs_a_fresh_matching_flow_and_the_existing_api_key() {
        let mut state = test_state().await;
        state
            .configure_oidc(ISSUER, "mwc", "test-secret", ORIGIN)
            .unwrap();
        let now = unix_timestamp().unwrap();
        let owner = state
            .database
            .create_user_with_initial_key(
                "account",
                KEY,
                false,
                ApiKeyScope::initial_key_defaults(false),
                now + 3600,
                now,
            )
            .await
            .unwrap();
        let app = router(Arc::new(state.clone()));
        let spoof = app
            .clone()
            .oneshot(
                Request::get("/api/v1/me")
                    .header("X-authentik-uid", owner.user_id.to_string())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(spoof.status(), StatusCode::UNAUTHORIZED);

        let callback_cookie = random_secret().unwrap();
        let callback = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/api/v1/auth/oidc/callback?code=fake&state={}",
                    random_secret().unwrap()
                ))
                .header(header::COOKIE, format!("{FLOW_COOKIE}={callback_cookie}"))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            callback.headers().get(header::LOCATION).unwrap(),
            "/?oidc=failed"
        );
        assert_eq!(
            callback.headers().get(header::REFERRER_POLICY).unwrap(),
            "no-referrer"
        );

        let confirm_cookie = random_secret().unwrap();
        save_flow(
            &state,
            &hash_secret(&confirm_cookie),
            &Flow::Confirm {
                action: Action::Bind,
                issuer: ISSUER.to_owned(),
                subject: "idp-user".to_owned(),
            },
            now,
        )
        .await
        .unwrap();
        let request = |origin: &str, action: &str| {
            Request::post("/api/v1/auth/oidc/confirm")
                .header(header::ORIGIN, origin)
                .header(header::AUTHORIZATION, format!("Bearer {KEY}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, format!("{FLOW_COOKIE}={confirm_cookie}"))
                .body(Body::from(format!(r#"{{"action":"{action}"}}"#)))
                .unwrap()
        };
        assert_eq!(
            app.clone()
                .oneshot(request("https://another.example", "bind"))
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            app.clone()
                .oneshot(request(ORIGIN, "unbind"))
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            app.clone()
                .oneshot(request(ORIGIN, "bind"))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );

        let cli = app
            .oneshot(
                Request::get("/api/v1/me")
                    .header(header::AUTHORIZATION, format!("Bearer {KEY}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cli.status(), StatusCode::OK);
    }
}
