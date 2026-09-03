use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::super::{Database, StorageError, identity::hash_token};
use crate::auth::ApiKeyScope;

mod audit;
mod listing;
mod persistence;
mod policy;
mod revocation;

use audit::{
    audit_admin_api_key_postgres, audit_admin_api_key_sqlite, audit_api_key_postgres,
    audit_api_key_sqlite,
};
pub use listing::{ApiKeyListStatus, ApiKeyPage};
use persistence::{
    ensure_key_capacity_postgres, ensure_key_capacity_sqlite, lock_user_postgres, lock_user_sqlite,
};
use policy::{generate_token, validate_api_key_name, validate_expiration, validate_scopes};
use revocation::{
    revoke_postgres, revoke_sqlite, revoke_user_key_postgres, revoke_user_key_sqlite,
};

pub(in crate::storage) use persistence::{insert_key_postgres, insert_key_sqlite};
pub(in crate::storage) use policy::token_prefix;
pub(crate) use policy::validate_api_key_policy;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ApiKeySummary {
    pub id: Uuid,
    pub name: String,
    pub prefix: String,
    pub last_used_at: Option<i64>,
    pub created_at: i64,
    pub scopes: Vec<ApiKeyScope>,
    pub expires_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

/// The outcome of a revocation attempt.  `remaining_active` is scoped to the
/// target user and is useful to callers deciding whether an audit record is due.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
pub struct ApiKeyRevokeResult {
    pub changed: bool,
    pub remaining_active: usize,
}

pub struct CreatedApiKey {
    pub summary: ApiKeySummary,
    pub token: String,
}

impl Database {
    pub async fn create_api_key(
        &self,
        user_id: Uuid,
        name: &str,
        scopes: Vec<ApiKeyScope>,
        expires_at: Option<i64>,
        now: i64,
    ) -> Result<CreatedApiKey, StorageError> {
        let name = validate_api_key_name(name)?;
        let scopes = validate_scopes(scopes)?;
        validate_expiration(expires_at, now)?;
        let token = generate_token()?;
        let summary = ApiKeySummary {
            id: Uuid::now_v7(),
            name,
            prefix: token_prefix(&token),
            last_used_at: None,
            created_at: now,
            scopes,
            expires_at,
            revoked_at: None,
        };
        let token_hash = hash_token(&token);
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                lock_user_sqlite(&mut transaction, installation_id.as_str(), user_id).await?;
                ensure_key_capacity_sqlite(&mut transaction, installation_id.as_str(), user_id)
                    .await?;
                insert_key_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    user_id,
                    &summary,
                    &token_hash,
                )
                .await?;
                audit_api_key_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    user_id,
                    summary.id,
                    "user.api_key.create",
                    now,
                )
                .await?;
                transaction.commit().await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                lock_user_postgres(&mut transaction, installation_id.as_str(), user_id).await?;
                ensure_key_capacity_postgres(&mut transaction, installation_id.as_str(), user_id)
                    .await?;
                insert_key_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    user_id,
                    &summary,
                    &token_hash,
                )
                .await?;
                audit_api_key_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    user_id,
                    summary.id,
                    "user.api_key.create",
                    now,
                )
                .await?;
                transaction.commit().await?;
            }
        }
        Ok(CreatedApiKey { summary, token })
    }

    pub async fn revoke_api_key(
        &self,
        user_id: Uuid,
        key_id: Uuid,
        now: i64,
    ) -> Result<ApiKeyRevokeResult, StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
                lock_user_sqlite(&mut transaction, installation_id.as_str(), user_id).await?;
                let result = revoke_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    user_id,
                    key_id,
                    now,
                )
                .await?;
                audit_api_key_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    user_id,
                    key_id,
                    "user.api_key.revoke",
                    now,
                )
                .await?;
                transaction.commit().await?;
                Ok(result)
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                lock_user_postgres(&mut transaction, installation_id.as_str(), user_id).await?;
                let result = revoke_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    user_id,
                    key_id,
                    now,
                )
                .await?;
                audit_api_key_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    user_id,
                    key_id,
                    "user.api_key.revoke",
                    now,
                )
                .await?;
                transaction.commit().await?;
                Ok(result)
            }
        }
    }

    /// Revokes a target user's key without applying the self-service recovery
    /// guard.  Missing, already-revoked, and mismatched keys are deliberately
    /// idempotent and return `changed: false`.
    pub async fn admin_revoke_api_key(
        &self,
        actor_user_id: Uuid,
        target_user_id: Uuid,
        key_id: Uuid,
        reason: &str,
        now: i64,
    ) -> Result<ApiKeyRevokeResult, StorageError> {
        if actor_user_id == target_user_id {
            return Err(StorageError::SelfApiKeyAdministration);
        }
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
                let result = revoke_user_key_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    target_user_id,
                    key_id,
                    now,
                )
                .await?;
                if result.changed {
                    audit_admin_api_key_sqlite(
                        &mut transaction,
                        installation_id.as_str(),
                        actor_user_id,
                        target_user_id,
                        key_id,
                        reason,
                        now,
                    )
                    .await?;
                }
                transaction.commit().await?;
                Ok(result)
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                let result = revoke_user_key_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    target_user_id,
                    key_id,
                    now,
                )
                .await?;
                if result.changed {
                    audit_admin_api_key_postgres(
                        &mut transaction,
                        installation_id.as_str(),
                        actor_user_id,
                        target_user_id,
                        key_id,
                        reason,
                        now,
                    )
                    .await?;
                }
                transaction.commit().await?;
                Ok(result)
            }
        }
    }
}
