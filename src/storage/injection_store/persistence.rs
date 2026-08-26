use base64::{Engine, engine::general_purpose::STANDARD};
use sqlx::{PgConnection, Row, SqliteConnection, postgres::PgRow, sqlite::SqliteRow};
use uuid::Uuid;

use crate::{
    crypto::{EncryptedEnvelope, EnvelopeCipher},
    injections::{InjectionItem, InjectionKind, InjectionScope, InjectionValue},
};

use super::{
    InjectionScopeRef, StoredInjectionSummary, aad, as_u32, as_u64, decode_array, decode_b64,
};
use crate::storage::StorageError;

pub(super) fn encrypted_sql(i: &str, s: &str, id: &str) -> String {
    format!(
        "SELECT scope, scope_id, key, kind, target, value_encoding, ciphertext, value_nonce, wrapped_data_key, key_nonce, sensitive, locked, version, file_mode, owner_name, group_name, template_selector, labels_json, updated_at FROM injection_items WHERE installation_id = {i} AND scope = {s} AND scope_id = {id} ORDER BY key"
    )
}

pub(super) fn summary_sql(i: &str, s: &str, id: &str) -> String {
    encrypted_sql(i, s, id)
}

pub(super) fn decode_summary_sqlite(
    row: SqliteRow,
) -> Result<StoredInjectionSummary, StorageError> {
    decode_summary(&row)
}

pub(super) fn decode_summary_postgres(row: PgRow) -> Result<StoredInjectionSummary, StorageError> {
    decode_summary(&row)
}

fn decode_summary<R: Row>(row: &R) -> Result<StoredInjectionSummary, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    let scope: String = row.try_get("scope")?;
    let scope_id: String = row.try_get("scope_id")?;
    let kind: String = row.try_get("kind")?;
    Ok(StoredInjectionSummary {
        key: row.try_get("key")?,
        kind: InjectionKind::from_database(&kind)
            .ok_or(StorageError::UnknownInjectionKind(kind))?,
        target: row.try_get("target")?,
        scope: InjectionScope::from_database(&scope)
            .ok_or(StorageError::UnknownInjectionScope(scope))?,
        scope_id: Uuid::parse_str(&scope_id)?,
        sensitive: row.try_get::<i64, _>("sensitive")? != 0,
        locked: row.try_get::<i64, _>("locked")? != 0,
        version: as_u64(row.try_get("version")?)?,
        file_mode: row
            .try_get::<Option<i64>, _>("file_mode")?
            .map(as_u32)
            .transpose()?,
        owner: row.try_get("owner_name")?,
        group: row.try_get("group_name")?,
        template_selector: row.try_get("template_selector")?,
        labels: serde_json::from_str(&row.try_get::<String, _>("labels_json")?)?,
        updated_at: row.try_get("updated_at")?,
    })
}

pub(super) fn decrypt_sqlite(
    cipher: &EnvelopeCipher,
    installation: &str,
    scope: InjectionScopeRef,
    row: SqliteRow,
) -> Result<InjectionItem, StorageError> {
    decrypt_row(cipher, installation, scope, &row)
}

pub(super) fn decrypt_postgres(
    cipher: &EnvelopeCipher,
    installation: &str,
    scope: InjectionScopeRef,
    row: PgRow,
) -> Result<InjectionItem, StorageError> {
    decrypt_row(cipher, installation, scope, &row)
}

fn decrypt_row<R: Row>(
    cipher: &EnvelopeCipher,
    installation: &str,
    scope: InjectionScopeRef,
    row: &R,
) -> Result<InjectionItem, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    let key: String = row.try_get("key")?;
    let version = as_u64(row.try_get("version")?)?;
    let envelope = EncryptedEnvelope {
        ciphertext: decode_b64(row.try_get("ciphertext")?)?,
        value_nonce: decode_array(row.try_get("value_nonce")?)?,
        wrapped_data_key: decode_b64(row.try_get("wrapped_data_key")?)?,
        key_nonce: decode_array(row.try_get("key_nonce")?)?,
    };
    let plaintext = cipher.decrypt(&envelope, &aad(installation, scope, &key, version))?;
    let encoding: String = row.try_get("value_encoding")?;
    let value = match encoding.as_str() {
        "utf8" => InjectionValue::Utf8(
            String::from_utf8(plaintext.to_vec())
                .map_err(|_| StorageError::InvalidEncryptedInjection)?,
        ),
        "base64" => InjectionValue::Base64(STANDARD.encode(plaintext.as_slice())),
        _ => return Err(StorageError::InvalidEncryptedInjection),
    };
    let kind: String = row.try_get("kind")?;
    Ok(InjectionItem {
        key,
        kind: InjectionKind::from_database(&kind)
            .ok_or(StorageError::UnknownInjectionKind(kind))?,
        target: row.try_get("target")?,
        value,
        sensitive: row.try_get::<i64, _>("sensitive")? != 0,
        locked: row.try_get::<i64, _>("locked")? != 0,
        version,
        file_mode: row
            .try_get::<Option<i64>, _>("file_mode")?
            .map(as_u32)
            .transpose()?,
        owner: row.try_get("owner_name")?,
        group: row.try_get("group_name")?,
        template_selector: row.try_get("template_selector")?,
        labels: serde_json::from_str(&row.try_get::<String, _>("labels_json")?)?,
    })
}

pub(super) async fn audit_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    scope: InjectionScopeRef,
    item: &InjectionItem,
    actor: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, 'injection.replace', ?6, ?7)").bind(Uuid::now_v7().to_string()).bind(installation).bind(actor.to_string()).bind((scope.scope == InjectionScope::Organization).then(|| scope.scope_id.to_string())).bind((scope.scope == InjectionScope::Workspace).then(|| scope.scope_id.to_string())).bind(serde_json::json!({"key": item.key, "scope": scope.scope.as_str(), "version": item.version}).to_string()).bind(now).execute(&mut *connection).await?;
    Ok(())
}

pub(super) async fn audit_postgres(
    connection: &mut PgConnection,
    installation: &str,
    scope: InjectionScopeRef,
    item: &InjectionItem,
    actor: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES ($1, $2, $3, $4, $5, 'injection.replace', $6, $7)").bind(Uuid::now_v7().to_string()).bind(installation).bind(actor.to_string()).bind((scope.scope == InjectionScope::Organization).then(|| scope.scope_id.to_string())).bind((scope.scope == InjectionScope::Workspace).then(|| scope.scope_id.to_string())).bind(serde_json::json!({"key": item.key, "scope": scope.scope.as_str(), "version": item.version}).to_string()).bind(now).execute(&mut *connection).await?;
    Ok(())
}

pub(super) fn summary(
    scope: InjectionScopeRef,
    item: &InjectionItem,
    now: i64,
) -> StoredInjectionSummary {
    StoredInjectionSummary {
        key: item.key.clone(),
        kind: item.kind,
        target: item.target.clone(),
        scope: scope.scope,
        scope_id: scope.scope_id,
        sensitive: item.sensitive,
        locked: item.locked,
        version: item.version,
        file_mode: item.file_mode,
        owner: item.owner.clone(),
        group: item.group.clone(),
        template_selector: item.template_selector.clone(),
        labels: item.labels.clone(),
        updated_at: now,
    }
}
