use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sqlx::Row;

use crate::{
    config::InstallationId,
    quota::Resources,
    templates::WorkspaceTemplateDocument,
    workspace_runtime::{
        WorkspaceNamespaceScope, WorkspaceRuntimeIdentity, WorkspaceRuntimeNamingScheme,
    },
    workspaces::AccessMode,
};
use uuid::Uuid;

use super::{Database, StorageError};

mod plugin_state;

const SNAPSHOT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseSnapshot {
    pub format_version: u32,
    pub schema_version: i64,
    pub installation_id: String,
    pub exported_at: i64,
    pub tables: BTreeMap<String, Vec<serde_json::Value>>,
}

impl Database {
    pub async fn export_snapshot(&self, now: i64) -> Result<DatabaseSnapshot, StorageError> {
        let Self::Sqlite {
            pool,
            installation_id,
        } = self
        else {
            return Err(StorageError::ExportRequiresSqlite);
        };
        // Keep every table, plugin blob and the schema version on one SQLite
        // read snapshot. In WAL mode writers may continue, while the exported
        // document cannot mix rows observed before and after a concurrent
        // transaction commits.
        let mut transaction = pool.begin().await?;
        let mut tables = BTreeMap::new();
        for (name, sql) in EXPORT_QUERIES {
            let rows = sqlx::query(sql)
                .bind(installation_id.as_str())
                .fetch_all(&mut *transaction)
                .await?;
            let values = rows
                .into_iter()
                .map(|row| {
                    let json = row.try_get::<String, _>("item")?;
                    serde_json::from_str(&json).map_err(StorageError::from)
                })
                .collect::<Result<Vec<_>, _>>()?;
            tables.insert((*name).to_owned(), values);
        }
        tables
            .extend(plugin_state::export_tables(&mut transaction, installation_id.as_str()).await?);
        let schema_version =
            sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM schema_migrations")
                .fetch_one(&mut *transaction)
                .await?;
        transaction.commit().await?;
        Ok(DatabaseSnapshot {
            format_version: SNAPSHOT_FORMAT_VERSION,
            schema_version,
            installation_id: installation_id.to_string(),
            exported_at: now,
            tables,
        })
    }

    pub async fn import_snapshot(&self, snapshot: &DatabaseSnapshot) -> Result<(), StorageError> {
        let Self::Postgres {
            pool,
            installation_id,
        } = self
        else {
            return Err(StorageError::ImportRequiresPostgres);
        };
        if snapshot.format_version != SNAPSHOT_FORMAT_VERSION {
            return Err(StorageError::UnsupportedSnapshotVersion(
                snapshot.format_version,
            ));
        }
        if snapshot.installation_id != installation_id.as_str() {
            return Err(StorageError::SnapshotInstallationMismatch {
                snapshot: snapshot.installation_id.clone(),
                configured: installation_id.to_string(),
            });
        }
        if snapshot.schema_version > self.schema_version().await? {
            return Err(StorageError::SnapshotSchemaTooNew(snapshot.schema_version));
        }

        let plugin_tables = plugin_state::prepare_import(snapshot)?;
        let mut transaction = pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("mwc:import:{installation_id}"))
            .execute(&mut *transaction)
            .await?;
        ensure_import_destination_empty(&mut transaction).await?;
        for table in IMPORT_ORDER {
            let rows = plugin_tables
                .as_ref()
                .and_then(|tables| tables.get(*table))
                .or_else(|| snapshot.tables.get(*table));
            let Some(rows) = rows else {
                if (*table == "workspace_injection_refs" && snapshot.schema_version < 7)
                    || (*table == "plugin_configurations" && snapshot.schema_version < 11)
                    || (plugin_state::is_plugin_table(table) && snapshot.schema_version < 13)
                    || (*table == "user_api_keys" && snapshot.schema_version < 14)
                    || (*table == "workspace_port_mappings" && snapshot.schema_version < 15)
                {
                    continue;
                }
                return Err(StorageError::SnapshotMissingTable((*table).to_owned()));
            };
            validate_snapshot_row_installations(table, rows, installation_id)?;
            if rows.is_empty() {
                continue;
            }
            let rows = normalize_snapshot_rows(table, rows, snapshot.schema_version)?;
            if *table == "workspaces" {
                validate_snapshot_workspace_rows(&rows, installation_id)?;
            }
            let json = serde_json::to_string(&rows)?;
            let sql = format!(
                "INSERT INTO {table} SELECT * FROM json_populate_recordset(NULL::{table}, $1::json)"
            );
            sqlx::query(&sql)
                .bind(json)
                .execute(&mut *transaction)
                .await?;
        }
        validate_imported_templates(&mut transaction, installation_id).await?;
        validate_imported_workspaces(&mut transaction, installation_id).await?;
        transaction.commit().await?;
        Ok(())
    }
}

async fn ensure_import_destination_empty(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), StorageError> {
    for table in IMPORT_DESTINATION_DATA_TABLES {
        let sql = format!("SELECT EXISTS (SELECT 1 FROM {table} LIMIT 1)");
        let has_rows: bool = sqlx::query_scalar(&sql)
            .fetch_one(&mut **transaction)
            .await?;
        if has_rows {
            return Err(StorageError::ImportDestinationNotEmpty);
        }
    }
    Ok(())
}

fn validate_snapshot_row_installations(
    table: &str,
    rows: &[Value],
    installation_id: &InstallationId,
) -> Result<(), StorageError> {
    if rows.iter().any(|row| {
        row.as_object()
            .and_then(|object| string_field(object, "installation_id"))
            != Some(installation_id.as_str())
    }) {
        return Err(StorageError::SnapshotRowInstallationMismatch {
            table: table.to_owned(),
        });
    }
    Ok(())
}

fn validate_snapshot_workspace_rows(
    rows: &[Value],
    installation_id: &InstallationId,
) -> Result<(), StorageError> {
    for row in rows {
        let object = row.as_object().ok_or(StorageError::InvalidWorkspace)?;
        let id = Uuid::parse_str(&required_workspace_string_field(object, "id")?)
            .map_err(|_| StorageError::InvalidWorkspace)?;
        let short_id = required_workspace_string_field(object, "short_id")?;
        let naming_scheme = WorkspaceRuntimeNamingScheme::from_database(
            &required_workspace_string_field(object, "runtime_naming_scheme")?,
        )
        .ok_or(StorageError::InvalidWorkspace)?;
        let namespace_scope = WorkspaceNamespaceScope::from_database(
            &required_workspace_string_field(object, "runtime_namespace_scope")?,
        )
        .ok_or(StorageError::InvalidWorkspace)?;
        let runtime = WorkspaceRuntimeIdentity {
            naming_scheme,
            namespace_scope,
            namespace: required_workspace_string_field(object, "runtime_namespace")?,
            resource_prefix: required_workspace_string_field(object, "runtime_resource_prefix")?,
            route_key: required_workspace_string_field(object, "runtime_route_key")?,
        };
        runtime
            .validate_for_workspace(installation_id, id, &short_id)
            .map_err(|_| StorageError::InvalidWorkspace)?;
    }
    Ok(())
}

async fn validate_imported_templates(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    installation_id: &InstallationId,
) -> Result<(), StorageError> {
    let rows =
        sqlx::query("SELECT template_yaml FROM workspace_templates WHERE installation_id = $1")
            .bind(installation_id.as_str())
            .fetch_all(&mut **transaction)
            .await?;
    for row in rows {
        let yaml: String = row.try_get("template_yaml")?;
        WorkspaceTemplateDocument::parse(&yaml).map_err(|_| StorageError::InvalidTemplate)?;
    }
    Ok(())
}

async fn validate_imported_workspaces(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    installation_id: &InstallationId,
) -> Result<(), StorageError> {
    let sql = format!(
        "SELECT {} FROM workspaces WHERE installation_id = $1",
        super::workspace_store::WORKSPACE_COLUMNS
    );
    let rows = sqlx::query(&sql)
        .bind(installation_id.as_str())
        .fetch_all(&mut **transaction)
        .await?;
    for row in rows {
        super::workspace_store::decode_postgres(row, installation_id)?;
    }
    Ok(())
}

fn normalize_snapshot_rows(
    table: &str,
    rows: &[Value],
    schema_version: i64,
) -> Result<Vec<Value>, StorageError> {
    match table {
        "user_api_keys" => Ok(normalize_snapshot_api_key_rows(rows, schema_version)),
        "workspace_templates" => normalize_template_rows(rows, schema_version),
        "workspaces" => normalize_workspace_rows(rows, schema_version),
        _ => Ok(rows.to_vec()),
    }
}

fn normalize_template_rows(
    rows: &[Value],
    schema_version: i64,
) -> Result<Vec<Value>, StorageError> {
    rows.iter()
        .cloned()
        .map(|row| normalize_template_row(row, schema_version, "template_yaml"))
        .collect()
}

fn normalize_workspace_rows(
    rows: &[Value],
    schema_version: i64,
) -> Result<Vec<Value>, StorageError> {
    rows.iter()
        .cloned()
        .map(|row| {
            let mut row = normalize_template_row(row, schema_version, "template_snapshot_yaml")?;
            if schema_version < 18
                && let Some(object) = row.as_object_mut()
            {
                add_legacy_workspace_runtime(object)?;
            }
            Ok(row)
        })
        .collect()
}

fn normalize_template_row(
    mut row: Value,
    schema_version: i64,
    yaml_key: &str,
) -> Result<Value, StorageError> {
    let Some(object) = row.as_object_mut() else {
        return Ok(row);
    };
    normalize_runtime_profile(object, schema_version);
    if schema_version < 10 || missing_yaml(object, yaml_key) {
        let yaml = legacy_template_yaml(object)?;
        object.insert(yaml_key.to_owned(), Value::String(yaml));
    }
    Ok(row)
}

fn normalize_runtime_profile(object: &mut Map<String, Value>, schema_version: i64) {
    let profile = object
        .entry("runtime_profile")
        .or_insert_with(|| Value::String("standard".to_owned()));
    if schema_version >= 9 {
        return;
    }
    let canonical = match profile.as_str() {
        Some("coder_rust_dev" | "coder_token_center_rust_dev") => Some("rust_dev"),
        Some("coder_node_dev") => Some("node_dev"),
        Some("coder_cluster_admin") => Some("maintainance"),
        _ => None,
    };
    if let Some(canonical) = canonical {
        *profile = Value::String(canonical.to_owned());
    }
}

fn missing_yaml(object: &Map<String, Value>, yaml_key: &str) -> bool {
    object
        .get(yaml_key)
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
}

fn legacy_template_yaml(object: &Map<String, Value>) -> Result<String, StorageError> {
    let access = string_field(object, "access_mode")
        .and_then(AccessMode::from_database)
        .ok_or(StorageError::InvalidTemplate)?;
    let resources = Resources {
        cpu_millis: unsigned_field(object, "cpu_millis")?,
        memory_mib: unsigned_field(object, "memory_mib")?,
        gpu_count: u32::try_from(unsigned_field(object, "gpu_count")?)
            .map_err(|_| StorageError::InvalidTemplate)?,
        disk_gib: unsigned_field(object, "disk_gib")?,
    };
    let spec = super::template_migration::from_legacy(
        required_string_field(object, "runtime_profile")?,
        required_string_field(object, "image")?,
        access,
        resources,
    )?;
    WorkspaceTemplateDocument::new(required_string_field(object, "name")?, spec)
        .to_yaml()
        .map_err(|_| StorageError::InvalidTemplate)
}

fn string_field<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    object.get(key).and_then(Value::as_str)
}

fn required_string_field<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, StorageError> {
    string_field(object, key).ok_or(StorageError::InvalidTemplate)
}

fn unsigned_field(object: &Map<String, Value>, key: &str) -> Result<u64, StorageError> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or(StorageError::InvalidTemplate)
}

fn add_legacy_workspace_runtime(object: &mut Map<String, Value>) -> Result<(), StorageError> {
    let installation_id = required_workspace_string_field(object, "installation_id")?;
    let short_id = required_workspace_string_field(object, "short_id")?;
    object.insert(
        "runtime_naming_scheme".to_owned(),
        Value::String("legacy_v1".to_owned()),
    );
    object.insert(
        "runtime_namespace_scope".to_owned(),
        Value::String("dedicated".to_owned()),
    );
    object.insert(
        "runtime_namespace".to_owned(),
        Value::String(format!("ws-{installation_id}-{short_id}")),
    );
    object.insert(
        "runtime_resource_prefix".to_owned(),
        Value::String("workspace".to_owned()),
    );
    object.insert("runtime_route_key".to_owned(), Value::String(short_id));
    Ok(())
}

fn required_workspace_string_field(
    object: &Map<String, Value>,
    key: &str,
) -> Result<String, StorageError> {
    string_field(object, key)
        .map(str::to_owned)
        .ok_or(StorageError::InvalidWorkspace)
}

/// Older snapshots either had no API-key grants or used a wildcard/unbounded
/// grant. Such token hashes must never become usable while importing data into
/// a current installation. Keep users and audit history, but omit unsafe key
/// records entirely.
fn normalize_snapshot_api_key_rows(
    rows: &[serde_json::Value],
    schema_version: i64,
) -> Vec<serde_json::Value> {
    if schema_version < 15 {
        return Vec::new();
    }

    rows.iter()
        .filter_map(|row| {
            let object = row.as_object()?;
            let scopes_json = object.get("scopes_json")?.as_str()?;
            let scopes = serde_json::from_str::<Vec<crate::auth::ApiKeyScope>>(scopes_json).ok()?;
            let has_expiry = object.get("expires_at").is_some_and(|value| value.is_i64());
            (!scopes.is_empty() && has_expiry).then(|| row.clone())
        })
        .collect()
}

const IMPORT_ORDER: &[&str] = &[
    "users",
    "user_api_keys",
    "plugin_packages",
    "plugin_assets",
    "plugin_catalog_metadata",
    "organizations",
    "organization_memberships",
    "organization_quotas",
    "user_quotas",
    "plugin_configurations",
    "image_policies",
    "workspace_templates",
    "workspaces",
    "workspace_port_mappings",
    "workspace_injection_refs",
    "audit_log",
    "injection_items",
    "webhook_subscriptions",
    "workspace_ssh_identities",
    "workspace_tombstones",
    "jobs",
    "events",
];

// A snapshot restore is replacement, not merge, semantics. Include both the
// authoritative tables in the snapshot and ephemeral tables that are reset by
// export so a partially initialized destination cannot be mistaken for empty.
const IMPORT_DESTINATION_DATA_TABLES: &[&str] = &[
    "users",
    "user_api_keys",
    "organizations",
    "organization_memberships",
    "organization_quotas",
    "user_quotas",
    "image_policies",
    "workspace_templates",
    "workspaces",
    "workspace_port_mappings",
    "workspace_port_mapping_tickets",
    "workspace_port_mapping_sessions",
    "workspace_injection_refs",
    "audit_log",
    "injection_items",
    "webhook_subscriptions",
    "workspace_ssh_identities",
    "workspace_tombstones",
    "jobs",
    "workspace_leases",
    "events",
    "web_shell_tickets",
    "idempotency_keys",
    "plugin_install_inspections",
    "plugin_packages",
    "plugin_assets",
    "plugin_catalog_metadata",
    "plugin_configurations",
    "plugin_ui_sessions",
];

const EXPORT_QUERIES: &[(&str, &str)] = &[
    (
        "users",
        "SELECT json_object('id', id, 'installation_id', installation_id, 'display_name', display_name, 'token_hash', token_hash, 'system_admin', system_admin, 'disabled', disabled, 'created_at', created_at, 'avatar_url', avatar_url) item FROM users WHERE installation_id = ?1 ORDER BY id",
    ),
    (
        "user_api_keys",
        "SELECT json_object('id', id, 'installation_id', installation_id, 'user_id', user_id, 'name', name, 'token_prefix', token_prefix, 'token_hash', token_hash, 'last_used_at', last_used_at, 'created_at', created_at, 'revoked_at', revoked_at, 'scopes_json', scopes_json, 'expires_at', expires_at) item FROM user_api_keys WHERE installation_id = ?1 ORDER BY user_id, created_at, id",
    ),
    (
        "organizations",
        "SELECT json_object('id', id, 'installation_id', installation_id, 'name', name, 'created_at', created_at) item FROM organizations WHERE installation_id = ?1 ORDER BY id",
    ),
    (
        "organization_memberships",
        "SELECT json_object('installation_id', installation_id, 'organization_id', organization_id, 'user_id', user_id, 'role', role, 'created_at', created_at) item FROM organization_memberships WHERE installation_id = ?1 ORDER BY organization_id, user_id",
    ),
    (
        "organization_quotas",
        "SELECT json_object('installation_id', installation_id, 'organization_id', organization_id, 'cpu_millis', cpu_millis, 'memory_mib', memory_mib, 'gpu_count', gpu_count, 'disk_gib', disk_gib, 'updated_at', updated_at) item FROM organization_quotas WHERE installation_id = ?1 ORDER BY organization_id",
    ),
    (
        "user_quotas",
        "SELECT json_object('installation_id', installation_id, 'user_id', user_id, 'cpu_millis', cpu_millis, 'memory_mib', memory_mib, 'gpu_count', gpu_count, 'disk_gib', disk_gib, 'updated_at', updated_at) item FROM user_quotas WHERE installation_id = ?1 ORDER BY user_id",
    ),
    (
        "plugin_configurations",
        "SELECT json_object('installation_id', installation_id, 'plugin_id', plugin_id, 'scope_key', scope_key, 'scope_kind', scope_kind, 'organization_id', organization_id, 'value_json', value_json, 'schema_digest', schema_digest, 'version', version, 'updated_by', updated_by, 'updated_at', updated_at) item FROM plugin_configurations WHERE installation_id = ?1 ORDER BY plugin_id, scope_key",
    ),
    (
        "image_policies",
        "SELECT json_object('installation_id', installation_id, 'image', image, 'contract_version', contract_version, 'enabled', enabled, 'created_at', created_at, 'updated_at', updated_at) item FROM image_policies WHERE installation_id = ?1 ORDER BY image",
    ),
    (
        "workspace_templates",
        "SELECT json_object('id', id, 'installation_id', installation_id, 'organization_id', organization_id, 'name', name, 'image', image, 'access_mode', access_mode, 'cpu_millis', cpu_millis, 'memory_mib', memory_mib, 'gpu_count', gpu_count, 'disk_gib', disk_gib, 'enabled', enabled, 'created_at', created_at, 'updated_at', updated_at, 'runtime_profile', runtime_profile, 'template_yaml', template_yaml) item FROM workspace_templates WHERE installation_id = ?1 ORDER BY id",
    ),
    (
        "workspaces",
        "SELECT json_object('id', id, 'installation_id', installation_id, 'short_id', short_id, 'organization_id', organization_id, 'owner_id', owner_id, 'name', name, 'template_id', template_id, 'image', image, 'access_mode', access_mode, 'state', state, 'cpu_millis', cpu_millis, 'memory_mib', memory_mib, 'gpu_count', gpu_count, 'disk_gib', disk_gib, 'generation', generation, 'created_at', created_at, 'updated_at', updated_at, 'deleted_at', deleted_at, 'runtime_profile', runtime_profile, 'template_snapshot_yaml', template_snapshot_yaml, 'runtime_naming_scheme', runtime_naming_scheme, 'runtime_namespace_scope', runtime_namespace_scope, 'runtime_namespace', runtime_namespace, 'runtime_resource_prefix', runtime_resource_prefix, 'runtime_route_key', runtime_route_key) item FROM workspaces WHERE installation_id = ?1 ORDER BY id",
    ),
    (
        "workspace_port_mappings",
        "SELECT json_object('id', id, 'installation_id', installation_id, 'organization_id', organization_id, 'workspace_id', workspace_id, 'internal_port', internal_port, 'display_name', display_name, 'created_by', created_by, 'created_at', created_at) item FROM workspace_port_mappings WHERE installation_id = ?1 ORDER BY workspace_id, created_at, id",
    ),
    (
        "workspace_injection_refs",
        "SELECT json_object('installation_id', installation_id, 'workspace_id', workspace_id, 'scope', scope, 'injection_key', injection_key, 'created_at', created_at) item FROM workspace_injection_refs WHERE installation_id = ?1 ORDER BY workspace_id, scope, injection_key",
    ),
    (
        "audit_log",
        "SELECT json_object('id', id, 'installation_id', installation_id, 'actor_user_id', actor_user_id, 'organization_id', organization_id, 'workspace_id', workspace_id, 'action', action, 'metadata_json', metadata_json, 'created_at', created_at) item FROM audit_log WHERE installation_id = ?1 ORDER BY id",
    ),
    (
        "injection_items",
        "SELECT json_object('id', id, 'installation_id', installation_id, 'scope', scope, 'scope_id', scope_id, 'key', key, 'kind', kind, 'target', target, 'value_encoding', value_encoding, 'ciphertext', ciphertext, 'value_nonce', value_nonce, 'wrapped_data_key', wrapped_data_key, 'key_nonce', key_nonce, 'sensitive', sensitive, 'locked', locked, 'version', version, 'file_mode', file_mode, 'owner_name', owner_name, 'group_name', group_name, 'template_selector', template_selector, 'labels_json', labels_json, 'created_by', created_by, 'created_at', created_at, 'updated_at', updated_at) item FROM injection_items WHERE installation_id = ?1 ORDER BY id",
    ),
    (
        "webhook_subscriptions",
        "SELECT json_object('id', id, 'installation_id', installation_id, 'organization_id', organization_id, 'url', url, 'event_prefix', event_prefix, 'ciphertext', ciphertext, 'value_nonce', value_nonce, 'wrapped_data_key', wrapped_data_key, 'key_nonce', key_nonce, 'enabled', enabled, 'created_by', created_by, 'created_at', created_at, 'updated_at', updated_at) item FROM webhook_subscriptions WHERE installation_id = ?1 ORDER BY id",
    ),
    (
        "workspace_ssh_identities",
        "SELECT json_object('installation_id', installation_id, 'workspace_id', workspace_id, 'public_key', public_key, 'fingerprint', fingerprint, 'ciphertext', ciphertext, 'value_nonce', value_nonce, 'wrapped_data_key', wrapped_data_key, 'key_nonce', key_nonce, 'created_at', created_at) item FROM workspace_ssh_identities WHERE installation_id = ?1 ORDER BY workspace_id",
    ),
    (
        "workspace_tombstones",
        "SELECT json_object('installation_id', installation_id, 'workspace_id', workspace_id, 'organization_id', organization_id, 'deleted_at', deleted_at) item FROM workspace_tombstones WHERE installation_id = ?1 ORDER BY workspace_id",
    ),
    (
        "jobs",
        "SELECT json_object('id', id, 'installation_id', installation_id, 'kind', kind, 'workspace_id', workspace_id, 'payload_json', payload_json, 'status', 'pending', 'available_at', available_at, 'lease_owner', NULL, 'lease_expires_at', NULL, 'attempts', attempts, 'created_at', created_at, 'updated_at', updated_at) item FROM jobs WHERE installation_id = ?1 AND status <> 'completed' ORDER BY id",
    ),
    (
        "events",
        "SELECT json_object('id', id, 'installation_id', installation_id, 'organization_id', organization_id, 'workspace_id', workspace_id, 'kind', kind, 'payload_json', payload_json, 'created_at', created_at) item FROM events WHERE installation_id = ?1 ORDER BY id",
    ),
];

#[cfg(test)]
mod tests {
    use super::{
        normalize_snapshot_rows, validate_snapshot_row_installations,
        validate_snapshot_workspace_rows,
    };
    use crate::{config::InstallationId, workspace_runtime::workspace_short_id_for};
    use uuid::Uuid;

    #[test]
    fn old_catalog_rows_receive_template_yaml() {
        for table in ["workspace_templates", "workspaces"] {
            let rows = vec![serde_json::json!({
                "id": "legacy", "installation_id": "snapshot-test", "short_id": "abc123",
                "name": "Legacy", "image": "registry.example/dev:latest",
                "access_mode": "internal", "cpu_millis": 1000, "memory_mib": 2048,
                "gpu_count": 0, "disk_gib": 20
            })];
            let normalized = normalize_snapshot_rows(table, &rows, 7).unwrap();
            assert_eq!(normalized[0]["runtime_profile"], "standard");
            let yaml_key = if table == "workspace_templates" {
                "template_yaml"
            } else {
                "template_snapshot_yaml"
            };
            assert!(
                normalized[0][yaml_key]
                    .as_str()
                    .unwrap()
                    .contains("WorkspaceTemplate")
            );
            assert!(rows[0].get("runtime_profile").is_none());
            if table == "workspaces" {
                assert_eq!(normalized[0]["runtime_naming_scheme"], "legacy_v1");
                assert_eq!(normalized[0]["runtime_namespace_scope"], "dedicated");
                assert_eq!(
                    normalized[0]["runtime_namespace"],
                    "ws-snapshot-test-abc123"
                );
                assert_eq!(normalized[0]["runtime_resource_prefix"], "workspace");
                assert_eq!(normalized[0]["runtime_route_key"], "abc123");
            }
        }
    }

    #[test]
    fn v14_api_keys_are_not_imported_without_explicit_bounded_grants() {
        let rows = vec![serde_json::json!({
            "id": "key", "installation_id": "test", "user_id": "user",
            "name": "Imported key", "token_prefix": "mwc_…", "token_hash": "hash",
            "last_used_at": null, "created_at": 1, "revoked_at": null
        })];
        let normalized = normalize_snapshot_rows("user_api_keys", &rows, 14).unwrap();
        assert!(normalized.is_empty());
    }

    #[test]
    fn unbounded_or_wildcard_snapshot_keys_are_not_imported() {
        let rows = vec![
            serde_json::json!({
                "id": "wildcard", "scopes_json": "[\"*\"]", "expires_at": 2_000_000_000
            }),
            serde_json::json!({
                "id": "unbounded", "scopes_json": "[\"read_workspace\"]", "expires_at": null
            }),
        ];
        let normalized = normalize_snapshot_rows("user_api_keys", &rows, 16).unwrap();
        assert!(normalized.is_empty());
    }

    #[test]
    fn v18_default_runtime_identity_is_rejected_before_insert() {
        let installation: InstallationId = "snapshot-test".parse().unwrap();
        let id = Uuid::parse_str("018f0000-0000-7000-8000-000000000001").unwrap();
        let short_id = workspace_short_id_for(id);
        let rows = vec![serde_json::json!({
            "id": id, "installation_id": installation.as_str(), "short_id": short_id,
            "runtime_naming_scheme": "legacy_v1", "runtime_namespace_scope": "dedicated",
            "runtime_namespace": "", "runtime_resource_prefix": "workspace",
            "runtime_route_key": ""
        })];
        assert!(validate_snapshot_workspace_rows(&rows, &installation).is_err());
    }

    #[test]
    fn imported_rows_must_belong_to_the_snapshot_installation() {
        let installation: InstallationId = "snapshot-test".parse().unwrap();
        let rows = vec![serde_json::json!({"installation_id": "other-installation"})];
        assert!(validate_snapshot_row_installations("users", &rows, &installation).is_err());
    }
}
