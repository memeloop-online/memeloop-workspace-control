use std::collections::BTreeMap;

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, Row, SqliteConnection};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    crypto::{EncryptedEnvelope, EnvelopeCipher},
    injections::{InjectionItem, InjectionKind, InjectionScope, InjectionValue},
};

use super::{Database, StorageError};

mod persistence;

use persistence::{
    audit_postgres, audit_sqlite, decode_summary_postgres, decode_summary_sqlite, decrypt_postgres,
    decrypt_sqlite, encrypted_sql, summary, summary_sql,
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
                let version = next_version_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    scope_ref,
                    &item.key,
                )
                .await?;
                item.version = version;
                let envelope = encrypt_item(cipher, installation_id.as_str(), scope_ref, &item)?;
                save_sqlite(
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
                let version = next_version_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    scope_ref,
                    &item.key,
                )
                .await?;
                item.version = version;
                let envelope = encrypt_item(cipher, installation_id.as_str(), scope_ref, &item)?;
                save_postgres(
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

pub(super) async fn insert_initial_workspace_injection_sqlite(
    connection: &mut SqliteConnection,
    cipher: &EnvelopeCipher,
    installation: &str,
    workspace_id: Uuid,
    item: &InjectionItem,
    actor: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    let mut item = item.clone();
    item.version = 1;
    let scope = InjectionScopeRef {
        scope: InjectionScope::Workspace,
        scope_id: workspace_id,
    };
    let envelope = encrypt_item(cipher, installation, scope, &item)?;
    save_sqlite(
        connection,
        installation,
        scope,
        &item,
        &envelope,
        actor,
        now,
    )
    .await
}

pub(super) async fn insert_initial_workspace_injection_postgres(
    connection: &mut PgConnection,
    cipher: &EnvelopeCipher,
    installation: &str,
    workspace_id: Uuid,
    item: &InjectionItem,
    actor: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    let mut item = item.clone();
    item.version = 1;
    let scope = InjectionScopeRef {
        scope: InjectionScope::Workspace,
        scope_id: workspace_id,
    };
    let envelope = encrypt_item(cipher, installation, scope, &item)?;
    save_postgres(
        connection,
        installation,
        scope,
        &item,
        &envelope,
        actor,
        now,
    )
    .await
}

async fn next_version_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    scope: InjectionScopeRef,
    key: &str,
) -> Result<u64, StorageError> {
    let row = sqlx::query("SELECT version FROM injection_items WHERE installation_id = ?1 AND scope = ?2 AND scope_id = ?3 AND key = ?4")
        .bind(installation).bind(scope.scope.as_str()).bind(scope.scope_id.to_string()).bind(key)
        .fetch_optional(&mut *connection).await?;
    next_version(
        row.map(|row| row.try_get::<i64, _>("version"))
            .transpose()?,
    )
}

async fn next_version_postgres(
    connection: &mut PgConnection,
    installation: &str,
    scope: InjectionScopeRef,
    key: &str,
) -> Result<u64, StorageError> {
    let row = sqlx::query("SELECT version FROM injection_items WHERE installation_id = $1 AND scope = $2 AND scope_id = $3 AND key = $4 FOR UPDATE")
        .bind(installation).bind(scope.scope.as_str()).bind(scope.scope_id.to_string()).bind(key)
        .fetch_optional(&mut *connection).await?;
    next_version(
        row.map(|row| row.try_get::<i64, _>("version"))
            .transpose()?,
    )
}

fn next_version(current: Option<i64>) -> Result<u64, StorageError> {
    let current = current
        .map_or(Ok(0), u64::try_from)
        .map_err(|_| StorageError::InvalidEncryptedInjection)?;
    current
        .checked_add(1)
        .ok_or(StorageError::InvalidEncryptedInjection)
}

fn encrypt_item(
    cipher: &EnvelopeCipher,
    installation: &str,
    scope: InjectionScopeRef,
    item: &InjectionItem,
) -> Result<EncryptedEnvelope, StorageError> {
    let plaintext = match &item.value {
        InjectionValue::Utf8(value) => value.as_bytes().to_vec(),
        InjectionValue::Base64(value) => STANDARD
            .decode(value)
            .map_err(|_| StorageError::InvalidEncryptedInjection)?,
    };
    Ok(cipher.encrypt(
        &plaintext,
        &aad(installation, scope, &item.key, item.version),
    )?)
}

async fn save_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    scope: InjectionScopeRef,
    item: &InjectionItem,
    envelope: &EncryptedEnvelope,
    actor: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query(&upsert_sqlite())
        .bind(Uuid::now_v7().to_string())
        .bind(installation)
        .bind(scope.scope.as_str())
        .bind(scope.scope_id.to_string())
        .bind(&item.key)
        .bind(item.kind.as_str())
        .bind(&item.target)
        .bind(encoding(&item.value))
        .bind(STANDARD.encode(&envelope.ciphertext))
        .bind(STANDARD.encode(envelope.value_nonce))
        .bind(STANDARD.encode(&envelope.wrapped_data_key))
        .bind(STANDARD.encode(envelope.key_nonce))
        .bind(i64::from(item.sensitive))
        .bind(i64::from(item.locked))
        .bind(as_i64(item.version)?)
        .bind(item.file_mode.map(i64::from))
        .bind(&item.owner)
        .bind(&item.group)
        .bind(&item.template_selector)
        .bind(serde_json::to_string(&item.labels)?)
        .bind(actor.to_string())
        .bind(now)
        .execute(&mut *connection)
        .await?;
    audit_sqlite(connection, installation, scope, item, actor, now).await
}

async fn save_postgres(
    connection: &mut PgConnection,
    installation: &str,
    scope: InjectionScopeRef,
    item: &InjectionItem,
    envelope: &EncryptedEnvelope,
    actor: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query(&upsert_postgres())
        .bind(Uuid::now_v7().to_string())
        .bind(installation)
        .bind(scope.scope.as_str())
        .bind(scope.scope_id.to_string())
        .bind(&item.key)
        .bind(item.kind.as_str())
        .bind(&item.target)
        .bind(encoding(&item.value))
        .bind(STANDARD.encode(&envelope.ciphertext))
        .bind(STANDARD.encode(envelope.value_nonce))
        .bind(STANDARD.encode(&envelope.wrapped_data_key))
        .bind(STANDARD.encode(envelope.key_nonce))
        .bind(i64::from(item.sensitive))
        .bind(i64::from(item.locked))
        .bind(as_i64(item.version)?)
        .bind(item.file_mode.map(i64::from))
        .bind(&item.owner)
        .bind(&item.group)
        .bind(&item.template_selector)
        .bind(serde_json::to_string(&item.labels)?)
        .bind(actor.to_string())
        .bind(now)
        .execute(&mut *connection)
        .await?;
    audit_postgres(connection, installation, scope, item, actor, now).await
}

fn upsert_sqlite() -> String {
    upsert(
        "?1", "?2", "?3", "?4", "?5", "?6", "?7", "?8", "?9", "?10", "?11", "?12", "?13", "?14",
        "?15", "?16", "?17", "?18", "?19", "?20", "?21", "?22",
    )
}
fn upsert_postgres() -> String {
    upsert(
        "$1", "$2", "$3", "$4", "$5", "$6", "$7", "$8", "$9", "$10", "$11", "$12", "$13", "$14",
        "$15", "$16", "$17", "$18", "$19", "$20", "$21", "$22",
    )
}

#[allow(clippy::too_many_arguments)]
fn upsert(
    id: &str,
    installation: &str,
    scope: &str,
    scope_id: &str,
    key: &str,
    kind: &str,
    target: &str,
    encoding: &str,
    ciphertext: &str,
    value_nonce: &str,
    wrapped: &str,
    key_nonce: &str,
    sensitive: &str,
    locked: &str,
    version: &str,
    mode: &str,
    owner: &str,
    group: &str,
    selector: &str,
    labels: &str,
    actor: &str,
    now: &str,
) -> String {
    format!(
        "INSERT INTO injection_items (id, installation_id, scope, scope_id, key, kind, target, value_encoding, ciphertext, value_nonce, wrapped_data_key, key_nonce, sensitive, locked, version, file_mode, owner_name, group_name, template_selector, labels_json, created_by, created_at, updated_at) VALUES ({id}, {installation}, {scope}, {scope_id}, {key}, {kind}, {target}, {encoding}, {ciphertext}, {value_nonce}, {wrapped}, {key_nonce}, {sensitive}, {locked}, {version}, {mode}, {owner}, {group}, {selector}, {labels}, {actor}, {now}, {now}) ON CONFLICT (installation_id, scope, scope_id, key) DO UPDATE SET kind = excluded.kind, target = excluded.target, value_encoding = excluded.value_encoding, ciphertext = excluded.ciphertext, value_nonce = excluded.value_nonce, wrapped_data_key = excluded.wrapped_data_key, key_nonce = excluded.key_nonce, sensitive = excluded.sensitive, locked = excluded.locked, version = excluded.version, file_mode = excluded.file_mode, owner_name = excluded.owner_name, group_name = excluded.group_name, template_selector = excluded.template_selector, labels_json = excluded.labels_json, updated_at = excluded.updated_at"
    )
}

fn aad(installation: &str, scope: InjectionScopeRef, key: &str, version: u64) -> Vec<u8> {
    format!(
        "mwc:v1:{installation}:{}:{}:{key}:{version}",
        scope.scope.as_str(),
        scope.scope_id
    )
    .into_bytes()
}
fn encoding(value: &InjectionValue) -> &'static str {
    match value {
        InjectionValue::Utf8(_) => "utf8",
        InjectionValue::Base64(_) => "base64",
    }
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
fn as_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::InvalidEncryptedInjection)
}
fn as_u64(value: i64) -> Result<u64, StorageError> {
    u64::try_from(value).map_err(|_| StorageError::InvalidEncryptedInjection)
}
fn as_u32(value: i64) -> Result<u32, StorageError> {
    u32::try_from(value).map_err(|_| StorageError::InvalidEncryptedInjection)
}
