use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{Client, Response, header};
use ring::signature::{RSA_PKCS1_2048_8192_SHA256, RsaPublicKeyComponents};
use serde::{Deserialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use tokio::sync::OnceCell;
use url::{Url, form_urlencoded};
use zeroize::Zeroizing;

const CALLBACK: &str = "/api/v1/auth/oidc/callback";
const MAX_DOCUMENT: usize = 64 * 1024;
const MAX_ID_TOKEN: usize = 16 * 1024;
const CLOCK_SKEW: i64 = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProviderError {
    Unavailable,
    InvalidAssertion,
}

type Result<T> = std::result::Result<T, ProviderError>;

pub(in crate::api) struct Provider {
    issuer: Url,
    discovery: Url,
    origin: String,
    redirect_uri: Url,
    client_id: String,
    client_secret: Zeroizing<String>,
    http: Client,
    endpoints: OnceCell<Endpoints>,
}

#[derive(Deserialize)]
struct Metadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    id_token_signing_alg_values_supported: Vec<String>,
    code_challenge_methods_supported: Vec<String>,
    token_endpoint_auth_methods_supported: Option<Vec<String>>,
}

struct Endpoints {
    authorize: Url,
    token: Url,
    jwks: Url,
    client_secret_post: bool,
}

#[derive(Deserialize)]
struct TokenResponse {
    id_token: String,
}

#[derive(Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

#[derive(Deserialize)]
struct Jwk {
    kid: Option<String>,
    kty: String,
    #[serde(rename = "use")]
    usage: Option<String>,
    key_ops: Option<Vec<String>>,
    alg: Option<String>,
    n: Option<String>,
    e: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignatureHeader {
    alg: String,
    kid: String,
    #[serde(rename = "typ")]
    token_type: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Audience {
    Single(String),
    List(Vec<String>),
}

#[derive(Deserialize)]
struct Claims {
    iss: String,
    sub: String,
    aud: Audience,
    azp: Option<String>,
    exp: i64,
    iat: i64,
    nbf: Option<i64>,
    nonce: String,
    auth_time: Option<i64>,
}

impl Provider {
    pub(in crate::api) fn new(
        issuer: &str,
        client_id: &str,
        client_secret: &str,
        origin: &str,
    ) -> std::result::Result<Self, String> {
        let issuer_url = https_url(issuer)
            .filter(|url| url.as_str() == issuer)
            .ok_or("OIDC issuer must be a canonical HTTPS URL")?;
        let redirect_uri = https_url(origin)
            .filter(|url| url.origin().ascii_serialization() == origin)
            .and_then(|url| url.join(CALLBACK).ok())
            .ok_or("OIDC browser origin must be a canonical HTTPS origin")?;
        if client_id.is_empty() || client_secret.is_empty() {
            return Err("OIDC confidential client ID and secret are required".into());
        }
        let discovery = Url::parse(&format!(
            "{}/.well-known/openid-configuration",
            issuer.trim_end_matches('/')
        ))
        .map_err(|_| "invalid OIDC discovery URL")?;
        let http = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| "could not initialize OIDC HTTP client")?;
        Ok(Self {
            issuer: issuer_url,
            discovery,
            origin: origin.to_owned(),
            redirect_uri,
            client_id: client_id.to_owned(),
            client_secret: Zeroizing::new(client_secret.to_owned()),
            http,
            endpoints: OnceCell::new(),
        })
    }

    pub(super) fn origin(&self) -> &str {
        &self.origin
    }

    pub(super) fn issuer(&self) -> &str {
        self.issuer.as_str()
    }

    pub(super) async fn authorization_url(
        &self,
        state: &str,
        nonce: &str,
        verifier: &str,
        force_login: bool,
    ) -> Result<Url> {
        let mut url = self.endpoints().await?.authorize.clone();
        self.write_authorization_query(&mut url, state, nonce, verifier, force_login);
        Ok(url)
    }

    fn write_authorization_query(
        &self,
        url: &mut Url,
        state: &str,
        nonce: &str,
        verifier: &str,
        force_login: bool,
    ) {
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut query = url.query_pairs_mut();
        query
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", self.redirect_uri.as_str())
            .append_pair("response_type", "code")
            .append_pair("scope", "openid")
            .append_pair("state", state)
            .append_pair("nonce", nonce)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");
        if force_login {
            query
                .append_pair("prompt", "login")
                .append_pair("max_age", "0");
        }
    }

    pub(super) async fn verified_subject(
        &self,
        code: &str,
        nonce: &str,
        verifier: &str,
        fresh_after: Option<i64>,
    ) -> Result<String> {
        if code.is_empty() || code.len() > 4096 {
            return Err(ProviderError::InvalidAssertion);
        }
        let endpoints = self.endpoints().await?;
        let request = self
            .http
            .post(endpoints.token.clone())
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        let request = {
            let mut body = form_urlencoded::Serializer::new(String::new());
            body.append_pair("grant_type", "authorization_code")
                .append_pair("code", code)
                .append_pair("redirect_uri", self.redirect_uri.as_str())
                .append_pair("code_verifier", verifier);
            if endpoints.client_secret_post {
                body.append_pair("client_id", &self.client_id)
                    .append_pair("client_secret", self.client_secret.as_str());
                request.body(body.finish())
            } else {
                request
                    .basic_auth(
                        form_escape(&self.client_id),
                        Some(form_escape(self.client_secret.as_str())),
                    )
                    .body(body.finish())
            }
        };
        let response = send(request).await?;
        if response.status().is_client_error() {
            return Err(ProviderError::InvalidAssertion);
        }
        let token: TokenResponse = document(response).await?;
        let token = CompactToken::parse(&token.id_token)?;
        // Only keys from configured discovery can verify an assertion. The JWT
        // header cannot name a URL or embed a replacement key.
        let keys = document(send(self.http.get(endpoints.jwks.clone())).await?).await?;
        let claims = token.verify(keys)?;
        let now = i64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| ProviderError::Unavailable)?
                .as_secs(),
        )
        .map_err(|_| ProviderError::Unavailable)?;
        claims.subject(self.issuer(), &self.client_id, nonce, now, fresh_after)
    }

    async fn endpoints(&self) -> Result<&Endpoints> {
        self.endpoints
            .get_or_try_init(|| async {
                let metadata = document(send(self.http.get(self.discovery.clone())).await?).await?;
                self.validate_metadata(metadata)
            })
            .await
    }

    fn validate_metadata(&self, metadata: Metadata) -> Result<Endpoints> {
        let has = |values: &[String], expected| values.iter().any(|value| value == expected);
        if metadata.issuer != self.issuer()
            || !has(&metadata.id_token_signing_alg_values_supported, "RS256")
            || !has(&metadata.code_challenge_methods_supported, "S256")
        {
            return Err(ProviderError::Unavailable);
        }
        let client_secret_post = match metadata.token_endpoint_auth_methods_supported {
            Some(methods) if has(&methods, "client_secret_post") => true,
            Some(methods) if !has(&methods, "client_secret_basic") => {
                return Err(ProviderError::Unavailable);
            }
            _ => false,
        };
        let endpoint = |value| {
            https_url(value)
                .filter(|url| url.origin() == self.issuer.origin())
                .ok_or(ProviderError::Unavailable)
        };
        Ok(Endpoints {
            authorize: endpoint(&metadata.authorization_endpoint)?,
            token: endpoint(&metadata.token_endpoint)?,
            jwks: endpoint(&metadata.jwks_uri)?,
            client_secret_post,
        })
    }
}

fn https_url(value: &str) -> Option<Url> {
    let url = Url::parse(value).ok()?;
    (url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none())
    .then_some(url)
}

fn form_escape(value: &str) -> String {
    form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

async fn send(request: reqwest::RequestBuilder) -> Result<Response> {
    // reqwest errors may carry a URL; expose only an error category.
    request.send().await.map_err(|_| ProviderError::Unavailable)
}

async fn document<T: DeserializeOwned>(mut response: Response) -> Result<T> {
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|size| size > MAX_DOCUMENT as u64)
    {
        return Err(ProviderError::Unavailable);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ProviderError::Unavailable)?
    {
        if chunk.len() > MAX_DOCUMENT - bytes.len() {
            return Err(ProviderError::Unavailable);
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| ProviderError::Unavailable)
}

struct CompactToken<'a> {
    input: &'a str,
    payload: &'a str,
    signature: Vec<u8>,
    kid: String,
}

impl<'a> CompactToken<'a> {
    fn parse(value: &'a str) -> Result<Self> {
        if value.len() > MAX_ID_TOKEN {
            return Err(ProviderError::InvalidAssertion);
        }
        let (input, signature) = value
            .rsplit_once('.')
            .ok_or(ProviderError::InvalidAssertion)?;
        let (header, payload) = input
            .split_once('.')
            .ok_or(ProviderError::InvalidAssertion)?;
        // A compact JWS contains exactly three Base64url segments.
        if payload.is_empty() || payload.contains('.') {
            return Err(ProviderError::InvalidAssertion);
        }
        let header: SignatureHeader = serde_json::from_slice(&decode(header)?)
            .map_err(|_| ProviderError::InvalidAssertion)?;
        if header.alg != "RS256"
            || header.kid.is_empty()
            || header.kid.len() > 255
            || header.token_type.as_deref().is_some_and(|typ| typ != "JWT")
        {
            return Err(ProviderError::InvalidAssertion);
        }
        Ok(Self {
            input,
            payload,
            signature: decode(signature)?,
            kid: header.kid,
        })
    }

    fn verify(self, jwks: Jwks) -> Result<Claims> {
        let mut matches = jwks
            .keys
            .into_iter()
            .filter(|key| key.kid.as_deref() == Some(self.kid.as_str()));
        let key = matches.next().ok_or(ProviderError::InvalidAssertion)?;
        if matches.next().is_some()
            || key.kty != "RSA"
            || key.usage.as_deref().is_some_and(|usage| usage != "sig")
            || key.alg.as_deref().is_some_and(|alg| alg != "RS256")
            || key
                .key_ops
                .is_some_and(|ops| !ops.iter().any(|op| op == "verify"))
        {
            return Err(ProviderError::InvalidAssertion);
        }
        let n = decode(key.n.as_deref().ok_or(ProviderError::InvalidAssertion)?)?;
        let e = decode(key.e.as_deref().ok_or(ProviderError::InvalidAssertion)?)?;
        RsaPublicKeyComponents { n, e }
            .verify(
                &RSA_PKCS1_2048_8192_SHA256,
                self.input.as_bytes(),
                &self.signature,
            )
            .map_err(|_| ProviderError::InvalidAssertion)?;
        serde_json::from_slice(&decode(self.payload)?).map_err(|_| ProviderError::InvalidAssertion)
    }
}

fn decode(value: &str) -> Result<Vec<u8>> {
    URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| ProviderError::InvalidAssertion)
}

impl Claims {
    fn subject(
        self,
        issuer: &str,
        client_id: &str,
        nonce: &str,
        now: i64,
        fresh_after: Option<i64>,
    ) -> Result<String> {
        let audience_matches = match &self.aud {
            Audience::Single(audience) => audience == client_id,
            // No additional audiences have been configured as trusted.
            Audience::List(audiences) => audiences.len() == 1 && audiences[0] == client_id,
        };
        if self.iss != issuer
            || !audience_matches
            || self.azp.as_deref().is_some_and(|azp| azp != client_id)
            || self.nonce != nonce
            || self.exp <= now
            || self.exp <= self.iat
            || self.iat <= 0
            || self.iat > now.saturating_add(CLOCK_SKEW)
            || self
                .nbf
                .is_some_and(|nbf| nbf > now.saturating_add(CLOCK_SKEW) || nbf >= self.exp)
            || fresh_after.is_some_and(|started_at| {
                self.auth_time.is_none_or(|auth_time| {
                    auth_time < started_at.saturating_sub(CLOCK_SKEW)
                        || auth_time > self.iat.saturating_add(CLOCK_SKEW)
                        || auth_time > now.saturating_add(CLOCK_SKEW)
                })
            })
            || self.sub.is_empty()
            || self.sub.len() > 255
            || !self.sub.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(ProviderError::InvalidAssertion);
        }
        Ok(self.sub)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Public key and assertion generated for this fixture; no private key is retained.
    const TEST_MODULUS: &str = "qwVXNMNGSTfj8Zr8YKqKvLtT0J2pQQ_Tvu2dokORT2iOffItOrvIN5sJHPSJ3WeW0DQuDovGxeG6UDQELvkYcNKzUuzDEZVw59irmscaTjVSlOrarat83BbU32PtB11LxmFgs9_xojt2BfuL-37qi0gj0gKqzKe5XtixBVWo7oD9qtMYXPLUHsCEIJCT3KvIWg0JAAch4FKxCL2A0yrwhe92o6PHt00Ju524hHEmgAtXjpCapL4yL3nOrFWSlG5oz2VV8_Acjrq_GzayZMHNpGxsBkAhIbw1ndkdWTnDM47680fmLQxUpLpv-ZMpojk8jkdfxAb8j0a9HtpTSMsNYQ";
    const TEST_JWT: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6InRlc3QtcnMyNTYiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2lkcC5leGFtcGxlL2FwcGxpY2F0aW9uL28vbXdjLyIsInN1YiI6InN0YWJsZS1zdWJqZWN0IiwiYXVkIjoibXdjLXRlc3QiLCJleHAiOjE3MDAwMDAzMDAsImlhdCI6MTcwMDAwMDAwMCwibm9uY2UiOiJicm93c2VyLW5vbmNlIiwiYXV0aF90aW1lIjoxNjk5OTk5OTk5fQ.e7qqf14aJSY-JihowmnwKUEf3-AKPtthQy1EULTRujAC1RcMKRiW33sl_ol79DsANiCsApPeOOeWbSd0c8afB1VhbAr-0sVhSglxqxxTj1KBtqgJoH7SfFq1uCIY6dgjoPokQOpbQcQeJDgLEZe-ZKWL-AE_HGckK4tbXd2CpTZDGhGEGuxJhg-iPUQZOi6Xlmlys2XTmQ5XzxF3BMarndK9-RvMpET5lSx_Vi66btg3OP1iykSdQhkc4aBY7z0GQbCVWiDaKPu-hWUXOiUJzP1LZC4NYgDAEDBFE0jEeeDTeZ0YE1Z9WKlWZDzQd_UATHRnZF0i-YitfoEYvqlgdA";
    const TEST_ISSUER: &str = "https://idp.example/application/o/mwc/";
    const TEST_TIME: i64 = 1_700_000_000;

    fn jwk() -> Jwk {
        Jwk {
            kid: Some("test-rs256".into()),
            kty: "RSA".into(),
            usage: Some("sig".into()),
            key_ops: None,
            alg: None,
            n: Some(TEST_MODULUS.into()),
            e: Some("AQAB".into()),
        }
    }

    fn claims() -> Claims {
        CompactToken::parse(TEST_JWT)
            .unwrap()
            .verify(Jwks { keys: vec![jwk()] })
            .unwrap()
    }

    #[test]
    fn exact_signed_fixture_and_tampering() {
        assert_eq!(
            claims().subject(
                TEST_ISSUER,
                "mwc-test",
                "browser-nonce",
                TEST_TIME,
                Some(TEST_TIME)
            ),
            Ok("stable-subject".into())
        );
        let (header, _) = TEST_JWT.split_once('.').unwrap();
        let (_, signature) = TEST_JWT.rsplit_once('.').unwrap();
        let changed = URL_SAFE_NO_PAD.encode(br#"{"sub":"someone-else"}"#);
        let tampered = format!("{header}.{changed}.{signature}");
        assert!(matches!(
            CompactToken::parse(&tampered)
                .unwrap()
                .verify(Jwks { keys: vec![jwk()] }),
            Err(ProviderError::InvalidAssertion)
        ));
        assert!(matches!(
            CompactToken::parse(TEST_JWT).unwrap().verify(Jwks {
                keys: vec![jwk(), jwk()]
            }),
            Err(ProviderError::InvalidAssertion)
        ));
    }

    #[test]
    fn rejects_untrusted_algorithms_and_key_sources() {
        let (_, payload_and_signature) = TEST_JWT.split_once('.').unwrap();
        for header in [
            r#"{"alg":"none","kid":"test-rs256"}"#,
            r#"{"alg":"HS256","kid":"test-rs256"}"#,
            r#"{"alg":"RS256","kid":"test-rs256","jku":"https://example.com/keys"}"#,
            r#"{"alg":"RS256","alg":"RS256","kid":"test-rs256"}"#,
        ] {
            let token = format!("{}.{payload_and_signature}", URL_SAFE_NO_PAD.encode(header));
            assert!(matches!(
                CompactToken::parse(&token),
                Err(ProviderError::InvalidAssertion)
            ));
        }
    }

    #[test]
    fn validates_exact_issuer_audience_nonce_expiry_and_fresh_authentication() {
        for (issuer, client, nonce, now, fresh_after) in [
            (
                "https://idp.example/application/o/mwc",
                "mwc-test",
                "browser-nonce",
                TEST_TIME,
                None,
            ),
            (
                TEST_ISSUER,
                "other-client",
                "browser-nonce",
                TEST_TIME,
                None,
            ),
            (TEST_ISSUER, "mwc-test", "other-nonce", TEST_TIME, None),
            (
                TEST_ISSUER,
                "mwc-test",
                "browser-nonce",
                TEST_TIME + 300,
                None,
            ),
            (
                TEST_ISSUER,
                "mwc-test",
                "browser-nonce",
                TEST_TIME + 60,
                Some(TEST_TIME + 60),
            ),
        ] {
            assert_eq!(
                claims().subject(issuer, client, nonce, now, fresh_after),
                Err(ProviderError::InvalidAssertion)
            );
        }
        let mut claims = claims();
        claims.auth_time = None;
        assert_eq!(
            claims.subject(
                TEST_ISSUER,
                "mwc-test",
                "browser-nonce",
                TEST_TIME,
                Some(TEST_TIME)
            ),
            Err(ProviderError::InvalidAssertion)
        );
    }

    #[test]
    fn discovery_pins_endpoint_origin_and_advertised_pkce() {
        let provider =
            Provider::new(TEST_ISSUER, "mwc-test", "synthetic", "https://mwc.example").unwrap();
        let metadata_json = serde_json::json!({
            "issuer": TEST_ISSUER,
            "authorization_endpoint": "https://idp.example/application/o/authorize/",
            "token_endpoint": "https://idp.example/application/o/token/",
            "jwks_uri": "https://idp.example/application/o/mwc/jwks/",
            "id_token_signing_alg_values_supported": ["RS256"],
            "code_challenge_methods_supported": ["S256"],
            "token_endpoint_auth_methods_supported": ["client_secret_post", "client_secret_basic"]
        });
        let mut changed = metadata_json.clone();
        changed["jwks_uri"] = "https://different-idp.example/keys".into();
        assert!(matches!(
            provider.validate_metadata(serde_json::from_value(changed).unwrap()),
            Err(ProviderError::Unavailable)
        ));
        let mut changed = metadata_json.clone();
        changed["code_challenge_methods_supported"] = serde_json::json!(["plain"]);
        assert!(matches!(
            provider.validate_metadata(serde_json::from_value(changed).unwrap()),
            Err(ProviderError::Unavailable)
        ));
        let mut endpoints = provider
            .validate_metadata(serde_json::from_value(metadata_json).unwrap())
            .unwrap();
        assert!(endpoints.client_secret_post);
        provider.write_authorization_query(
            &mut endpoints.authorize,
            "browser-state",
            "browser-nonce",
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
            true,
        );
        let fields: std::collections::HashMap<_, _> = endpoints.authorize.query_pairs().collect();
        assert_eq!(
            fields["redirect_uri"],
            "https://mwc.example/api/v1/auth/oidc/callback"
        );
        assert_eq!(fields["scope"], "openid");
        assert_eq!(fields["code_challenge_method"], "S256");
        assert_eq!(
            fields["code_challenge"],
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert_eq!(fields["state"], "browser-state");
        assert_eq!(fields["nonce"], "browser-nonce");
        assert_eq!(fields["prompt"], "login");
        assert_eq!(fields["max_age"], "0");
        assert!(!fields.contains_key("client_secret"));
    }
}
