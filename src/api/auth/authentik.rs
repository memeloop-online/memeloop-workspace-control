use std::{collections::HashMap, sync::LazyLock, time::Duration};

use axum::{
    extract::Request,
    http::{HeaderMap, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse as _, Response},
};
use uuid::Uuid;

const OUTPOST_PATH: &str = "/outpost.goauthentik.io/auth/traefik";

struct BrowserAuth {
    origin: String,
    host: String,
    endpoint: reqwest::Url,
    users: HashMap<String, Uuid>,
    client: reqwest::Client,
}

static CONFIG: LazyLock<Result<Option<BrowserAuth>, String>> = LazyLock::new(BrowserAuth::load);

impl BrowserAuth {
    fn load() -> Result<Option<Self>, String> {
        let Some(origin) = std::env::var_os("MWC_AUTHENTIK_ORIGIN") else {
            return Ok(None);
        };
        let origin = origin.into_string().map_err(|_| "invalid browser origin")?;
        let origin_url = reqwest::Url::parse(&origin).map_err(|_| "invalid browser origin")?;
        if origin_url.scheme() != "https" || origin_url.origin().ascii_serialization() != origin {
            return Err("browser origin must be a canonical HTTPS origin".into());
        }
        let host = origin
            .strip_prefix("https://")
            .ok_or("invalid browser origin")?
            .to_owned();
        let outpost =
            std::env::var("MWC_AUTHENTIK_OUTPOST").map_err(|_| "missing authentik outpost")?;
        let outpost_url = reqwest::Url::parse(&outpost).map_err(|_| "invalid authentik outpost")?;
        if !matches!(outpost_url.scheme(), "http" | "https")
            || outpost_url.origin().ascii_serialization() != outpost
        {
            return Err("authentik outpost must be a canonical service origin".into());
        }
        let users = std::env::var("MWC_AUTHENTIK_USERS_JSON")
            .map_err(|_| "missing authentik user bindings")?;
        let users: HashMap<String, Uuid> =
            serde_json::from_str(&users).map_err(|_| "invalid authentik user bindings")?;
        if users.is_empty() || users.keys().any(|uid| !valid_uid(uid)) {
            return Err("authentik user bindings require stable authentik UIDs".into());
        }
        Ok(Some(Self {
            origin,
            host,
            endpoint: outpost_url
                .join(OUTPOST_PATH)
                .map_err(|_| "invalid authentik endpoint")?,
            users,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .timeout(Duration::from_secs(3))
                .build()
                .map_err(|_| "could not initialize authentik HTTP client")?,
        }))
    }
}

fn valid_uid(uid: &str) -> bool {
    uid.len() == 64
        && uid
            .bytes()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, b'a'..=b'f'))
}

pub(in crate::api) fn enabled() -> bool {
    !matches!(&*CONFIG, Ok(None))
}

pub(in crate::api) async fn csrf(request: Request, next: Next) -> Response {
    if let Ok(Some(config)) = &*CONFIG
        && request.headers().contains_key(header::COOKIE)
        && !request.headers().contains_key(header::AUTHORIZATION)
        && !matches!(
            *request.method(),
            Method::GET | Method::HEAD | Method::OPTIONS
        )
        && request
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            != Some(config.origin.as_str())
    {
        return super::ApiError::Forbidden.into_response();
    }
    next.run(request).await
}

pub(super) async fn browser_user(headers: &HeaderMap) -> Result<Option<Uuid>, super::ApiError> {
    let config = match &*CONFIG {
        Ok(Some(config)) => config,
        Ok(None) => return Ok(None),
        Err(reason) => {
            tracing::error!(%reason, "invalid browser identity configuration");
            return Err(super::ApiError::KubernetesUnavailable);
        }
    };
    let Some(cookie) = headers.get(header::COOKIE) else {
        return Ok(None);
    };
    let response = config
        .client
        .get(config.endpoint.clone())
        .header(header::HOST, config.host.as_str())
        .header(header::COOKIE, cookie)
        .header("x-forwarded-host", config.host.as_str())
        .header("x-forwarded-proto", "https")
        .header("x-forwarded-uri", "/")
        .send()
        .await
        .map_err(|_| super::ApiError::KubernetesUnavailable)?;
    if response.status().is_server_error() || response.status() == StatusCode::NOT_FOUND {
        return Err(super::ApiError::KubernetesUnavailable);
    }
    if response.status() != StatusCode::OK {
        return Ok(None);
    }
    // Trust only the verified outpost's response, never same-named incoming headers.
    let uid = response
        .headers()
        .get("x-authentik-uid")
        .and_then(|v| v.to_str().ok());
    uid.map(|uid| {
        config
            .users
            .get(uid)
            .copied()
            .ok_or(super::ApiError::Forbidden)
    })
    .transpose()
}
