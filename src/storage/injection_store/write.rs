use base64::{Engine, engine::general_purpose::STANDARD};
use sqlx::{PgConnection, Row, SqliteConnection};
use uuid::Uuid;

use crate::{
    crypto::{EncryptedEnvelope, EnvelopeCipher},
    injections::{InjectionItem, InjectionScope, InjectionValue},
    storage::StorageError,
};

use super::{
    InjectionScopeRef, aad,
    persistence::{audit_postgres, audit_sqlite},
};

pub(crate) async fn insert_initial_workspace_injection_sqlite(
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

pub(crate) async fn insert_initial_workspace_injection_postgres(
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

pub(super) async fn next_version_sqlite(
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

pub(super) async fn next_version_postgres(
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

pub(super) fn encrypt_item(
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

pub(super) async fn save_sqlite(
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

pub(super) async fn save_postgres(
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
    upsert(&[
        "?1", "?2", "?3", "?4", "?5", "?6", "?7", "?8", "?9", "?10", "?11", "?12", "?13", "?14",
        "?15", "?16", "?17", "?18", "?19", "?20", "?21", "?22",
    ])
}

fn upsert_postgres() -> String {
    upsert(&[
        "$1", "$2", "$3", "$4", "$5", "$6", "$7", "$8", "$9", "$10", "$11", "$12", "$13", "$14",
        "$15", "$16", "$17", "$18", "$19", "$20", "$21", "$22",
    ])
}

fn upsert(p: &[&str; 22]) -> String {
    let [
        id,
        installation,
        scope,
        scope_id,
        key,
        kind,
        target,
        encoding,
        ciphertext,
        value_nonce,
        wrapped,
        key_nonce,
        sensitive,
        locked,
        version,
        mode,
        owner,
        group,
        selector,
        labels,
        actor,
        now,
    ] = *p;
    format!(
        "INSERT INTO injection_items (id, installation_id, scope, scope_id, key, kind, target, value_encoding, ciphertext, value_nonce, wrapped_data_key, key_nonce, sensitive, locked, version, file_mode, owner_name, group_name, template_selector, labels_json, created_by, created_at, updated_at) VALUES ({id}, {installation}, {scope}, {scope_id}, {key}, {kind}, {target}, {encoding}, {ciphertext}, {value_nonce}, {wrapped}, {key_nonce}, {sensitive}, {locked}, {version}, {mode}, {owner}, {group}, {selector}, {labels}, {actor}, {now}, {now}) ON CONFLICT (installation_id, scope, scope_id, key) DO UPDATE SET kind = excluded.kind, target = excluded.target, value_encoding = excluded.value_encoding, ciphertext = excluded.ciphertext, value_nonce = excluded.value_nonce, wrapped_data_key = excluded.wrapped_data_key, key_nonce = excluded.key_nonce, sensitive = excluded.sensitive, locked = excluded.locked, version = excluded.version, file_mode = excluded.file_mode, owner_name = excluded.owner_name, group_name = excluded.group_name, template_selector = excluded.template_selector, labels_json = excluded.labels_json, updated_at = excluded.updated_at"
    )
}

fn encoding(value: &InjectionValue) -> &'static str {
    match value {
        InjectionValue::Utf8(_) => "utf8",
        InjectionValue::Base64(_) => "base64",
    }
}

fn as_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::InvalidEncryptedInjection)
}
