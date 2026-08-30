use std::collections::BTreeMap;

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    crypto::EnvelopeCipher,
    injections::{InjectionItem, InjectionKind, InjectionScope},
};

use super::{Database, StorageError};

mod deletion;
mod persistence;
mod write;

use persistence::{
    decode_summary_postgres, decode_summary_sqlite, decrypt_postgres, decrypt_sqlite,
    encrypted_sql, summary, summary_sql,
};
pub(super) use write::{
    insert_initial_workspace_injection_postgres, insert_initial_workspace_injection_sqlite,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, IntoParams, ToSchema)]
pub struct InjectionScopeRef {
    pub scope: InjectionScope,
    pub scope_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct StoredInjectionSummary {
    pub key: String,
    pub kind: InjectionKind,
    pub target: String,
    pub scope: InjectionScope,
    pub scope_id: Uuid,
    pub sensitive: bool,
    pub locked: bool,
    pub version: u64,
    pub file_mode: Option<u32>,
    pub owner: Option<String>,
    pub group: Option<String>,
    pub template_selector: Option<String>,
    pub labels: BTreeMap<String, String>,
    pub updated_at: i64,
}

impl Database {
    pub async fn replace_injection(
        &self,
        cipher: &EnvelopeCipher,
        scope_ref: InjectionScopeRef,
        mut item: InjectionItem,
        actor_user_id: Uuid,
        now: i64,
    ) -> Result<StoredInjectionSummary, StorageError> {
        if item.locked && scope_ref.scope != InjectionScope::Organization {
            return Err(StorageError::InvalidInjectionLock);
        }
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                let version = write::next_version_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    scope_ref,
                    &item.key,
                )
                .await?;
                item.version = version;
                let envelope =
                    write::encrypt_item(cipher, installation_id.as_str(), scope_ref, &item)?;
                write::save_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    scope_ref,
                    &item,
                    &envelope,
                    actor_user_id,
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
                let version = write::next_version_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    scope_ref,
                    &item.key,
                )
                .await?;
                item.version = version;
                let envelope =
                    write::encrypt_item(cipher, installation_id.as_str(), scope_ref, &item)?;
                write::save_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    scope_ref,
                    &item,
                    &envelope,
                    actor_user_id,
                    now,
                )
                .await?;
                transaction.commit().await?;
            }
        }
        Ok(summary(scope_ref, &item, now))
    }

    pub async fn list_injection_summaries(
        &self,
        scope_ref: InjectionScopeRef,
    ) -> Result<Vec<StoredInjectionSummary>, StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(&summary_sql("?1", "?2", "?3"))
                .bind(installation_id.as_str())
                .bind(scope_ref.scope.as_str())
                .bind(scope_ref.scope_id.to_string())
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(decode_summary_sqlite)
                .collect(),
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(&summary_sql("$1", "$2", "$3"))
                .bind(installation_id.as_str())
                .bind(scope_ref.scope.as_str())
                .bind(scope_ref.scope_id.to_string())
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(decode_summary_postgres)
                .collect(),
        }
    }

    pub async fn load_injections(
        &self,
        cipher: &EnvelopeCipher,
        scope_ref: InjectionScopeRef,
    ) -> Result<Vec<InjectionItem>, StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let rows = sqlx::query(&encrypted_sql("?1", "?2", "?3"))
                    .bind(installation_id.as_str())
                    .bind(scope_ref.scope.as_str())
                    .bind(scope_ref.scope_id.to_string())
                    .fetch_all(pool)
                    .await?;
                rows.into_iter()
                    .map(|row| decrypt_sqlite(cipher, installation_id.as_str(), scope_ref, row))
                    .collect()
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let rows = sqlx::query(&encrypted_sql("$1", "$2", "$3"))
                    .bind(installation_id.as_str())
                    .bind(scope_ref.scope.as_str())
                    .bind(scope_ref.scope_id.to_string())
                    .fetch_all(pool)
                    .await?;
                rows.into_iter()
                    .map(|row| decrypt_postgres(cipher, installation_id.as_str(), scope_ref, row))
                    .collect()
            }
        }
    }
}

fn aad(installation: &str, scope: InjectionScopeRef, key: &str, version: u64) -> Vec<u8> {
    format!(
        "mwc:v1:{installation}:{}:{}:{key}:{version}",
        scope.scope.as_str(),
        scope.scope_id
    )
    .into_bytes()
}
fn decode_b64(value: String) -> Result<Vec<u8>, StorageError> {
    STANDARD
        .decode(value)
        .map_err(|_| StorageError::InvalidEncryptedInjection)
}
fn decode_array(value: String) -> Result<[u8; 12], StorageError> {
    decode_b64(value)?
        .try_into()
        .map_err(|_| StorageError::InvalidEncryptedInjection)
}
fn as_u64(value: i64) -> Result<u64, StorageError> {
    u64::try_from(value).map_err(|_| StorageError::InvalidEncryptedInjection)
}
fn as_u32(value: i64) -> Result<u32, StorageError> {
    u32::try_from(value).map_err(|_| StorageError::InvalidEncryptedInjection)
}
