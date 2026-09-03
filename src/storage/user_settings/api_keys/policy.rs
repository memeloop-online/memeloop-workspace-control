use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

use crate::{auth::ApiKeyScope, storage::StorageError};

/// Validate the policy shared by self-service keys and administrator-provisioned
/// initial keys. New keys must always be explicitly scoped and time-bounded;
/// `Wildcard` remains readable only for keys created before this policy existed.
pub(crate) fn validate_api_key_policy(
    scopes: Vec<ApiKeyScope>,
    expires_at: Option<i64>,
    now: i64,
) -> Result<Vec<ApiKeyScope>, StorageError> {
    let scopes = validate_scopes(scopes)?;
    validate_expiration(expires_at, now)?;
    Ok(scopes)
}

pub(super) fn validate_scopes(scopes: Vec<ApiKeyScope>) -> Result<Vec<ApiKeyScope>, StorageError> {
    if scopes.is_empty()
        || scopes
            .iter()
            .any(|scope| matches!(scope, ApiKeyScope::Wildcard))
    {
        return Err(StorageError::InvalidApiKey);
    }
    let mut scopes = scopes;
    scopes.sort_by_key(|scope| *scope as u8);
    scopes.dedup();
    Ok(scopes)
}

const MAX_API_KEY_LIFETIME_SECONDS: i64 = 365 * 24 * 60 * 60;

pub(super) fn validate_expiration(expires_at: Option<i64>, now: i64) -> Result<(), StorageError> {
    let Some(expires_at) = expires_at else {
        return Err(StorageError::InvalidApiKey);
    };
    if expires_at <= now || expires_at.saturating_sub(now) > MAX_API_KEY_LIFETIME_SECONDS {
        return Err(StorageError::InvalidApiKey);
    }
    Ok(())
}

pub(super) fn validate_api_key_name(name: &str) -> Result<String, StorageError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err(StorageError::InvalidApiKey);
    }
    Ok(name.to_owned())
}

pub(super) fn generate_token() -> Result<String, StorageError> {
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random).map_err(|_| StorageError::RandomSource)?;
    Ok(format!("mwc_{}", URL_SAFE_NO_PAD.encode(random)))
}

pub(in crate::storage) fn token_prefix(token: &str) -> String {
    let visible = token.chars().take(12).collect::<String>();
    format!("{visible}…")
}
