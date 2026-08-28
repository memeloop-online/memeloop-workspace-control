use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::{
    quota::Resources,
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::AccessMode,
};

use super::{Database, StorageError};

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
        let mut tables = BTreeMap::new();
        for (name, sql) in EXPORT_QUERIES {
            let rows = sqlx::query(sql)
                .bind(installation_id.as_str())
                .fetch_all(pool)
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
        Ok(DatabaseSnapshot {
            format_version: SNAPSHOT_FORMAT_VERSION,
            schema_version: self.schema_version().await?,
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

        let mut transaction = pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("mwc:import:{installation_id}"))
            .execute(&mut *transaction)
            .await?;
        let existing: i64 = sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM users) + (SELECT COUNT(*) FROM organizations) + \
            (SELECT COUNT(*) FROM workspaces) + (SELECT COUNT(*) FROM injection_items)",
        )
        .fetch_one(&mut *transaction)
        .await?;
        if existing != 0 {
            return Err(StorageError::ImportDestinationNotEmpty);
        }
        for table in IMPORT_ORDER {
            let Some(rows) = snapshot.tables.get(*table) else {
                if *table == "workspace_injection_refs" && snapshot.schema_version < 7 {
                    continue;
                }
                return Err(StorageError::SnapshotMissingTable((*table).to_owned()));
            };
            if rows.is_empty() {
                continue;
            }
            let rows = normalize_snapshot_rows(table, rows, snapshot.schema_version)?;
            let json = serde_json::to_string(&rows)?;
            let sql = format!(
                "INSERT INTO {table} SELECT * FROM json_populate_recordset(NULL::{table}, $1::json)"
            );
            sqlx::query(&sql)
                .bind(json)
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }
}

fn normalize_snapshot_rows(
    table: &str,
    rows: &[serde_json::Value],
    schema_version: i64,
) -> Result<Vec<serde_json::Value>, StorageError> {
    if !matches!(table, "workspace_templates" | "workspaces") {
        return Ok(rows.to_vec());
    }
    rows.iter()
        .cloned()
        .map(|mut row| {
            if let Some(object) = row.as_object_mut() {
                let profile = object
                    .entry("runtime_profile")
                    .or_insert_with(|| serde_json::Value::String("standard".to_owned()));
                if schema_version < 9 {
                    let canonical = match profile.as_str() {
                        Some("coder_rust_dev" | "coder_token_center_rust_dev") => Some("rust_dev"),
                        Some("coder_node_dev") => Some("node_dev"),
                        Some("coder_cluster_admin") => Some("maintainance"),
                        _ => None,
                    };
                    if let Some(canonical) = canonical {
                        *profile = serde_json::Value::String(canonical.to_owned());
                    }
                }
                let yaml_key = if table == "workspace_templates" {
                    "template_yaml"
                } else {
                    "template_snapshot_yaml"
                };
                if schema_version < 10
                    || object
                        .get(yaml_key)
                        .and_then(|value| value.as_str())
                        .is_none_or(str::is_empty)
                {
                    let access = object
                        .get("access_mode")
                        .and_then(|value| value.as_str())
                        .and_then(AccessMode::from_database)
                        .ok_or(StorageError::InvalidTemplate)?;
                    let unsigned = |key: &str| {
                        object
                            .get(key)
                            .and_then(|value| value.as_u64())
                            .ok_or(StorageError::InvalidTemplate)
                    };
                    let resources = Resources {
                        cpu_millis: unsigned("cpu_millis")?,
                        memory_mib: unsigned("memory_mib")?,
                        gpu_count: u32::try_from(unsigned("gpu_count")?)
                            .map_err(|_| StorageError::InvalidTemplate)?,
                        disk_gib: unsigned("disk_gib")?,
                    };
                    let profile = object
                        .get("runtime_profile")
                        .and_then(|value| value.as_str())
                        .ok_or(StorageError::InvalidTemplate)?;
                    let image = object
                        .get("image")
                        .and_then(|value| value.as_str())
                        .ok_or(StorageError::InvalidTemplate)?;
                    let name = object
                        .get("name")
                        .and_then(|value| value.as_str())
                        .ok_or(StorageError::InvalidTemplate)?;
                    let spec =
                        WorkspaceTemplateSpec::from_legacy(profile, image, access, resources)
                            .map_err(|_| StorageError::InvalidTemplate)?;
                    let yaml = WorkspaceTemplateDocument::new(name, spec)
                        .to_yaml()
                        .map_err(|_| StorageError::InvalidTemplate)?;
                    object.insert(yaml_key.to_owned(), serde_json::Value::String(yaml));
                }
            }
            Ok(row)
        })
        .collect()
}

const IMPORT_ORDER: &[&str] = &[
    "users",
    "organizations",
    "organization_memberships",
    "organization_quotas",
    "user_quotas",
    "image_policies",
    "workspace_templates",
    "workspaces",
    "workspace_injection_refs",
    "audit_log",
    "injection_items",
    "webhook_subscriptions",
    "workspace_ssh_identities",
    "workspace_tombstones",
    "jobs",
    "events",
];

const EXPORT_QUERIES: &[(&str, &str)] = &[
    (
        "users",
        "SELECT json_object('id', id, 'installation_id', installation_id, 'display_name', display_name, 'token_hash', token_hash, 'system_admin', system_admin, 'disabled', disabled, 'created_at', created_at) item FROM users WHERE installation_id = ?1 ORDER BY id",
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
        "image_policies",
        "SELECT json_object('installation_id', installation_id, 'image', image, 'contract_version', contract_version, 'enabled', enabled, 'created_at', created_at, 'updated_at', updated_at) item FROM image_policies WHERE installation_id = ?1 ORDER BY image",
    ),
    (
        "workspace_templates",
        "SELECT json_object('id', id, 'installation_id', installation_id, 'organization_id', organization_id, 'name', name, 'image', image, 'access_mode', access_mode, 'cpu_millis', cpu_millis, 'memory_mib', memory_mib, 'gpu_count', gpu_count, 'disk_gib', disk_gib, 'enabled', enabled, 'created_at', created_at, 'updated_at', updated_at, 'runtime_profile', runtime_profile, 'template_yaml', template_yaml) item FROM workspace_templates WHERE installation_id = ?1 ORDER BY id",
    ),
    (
        "workspaces",
        "SELECT json_object('id', id, 'installation_id', installation_id, 'short_id', short_id, 'organization_id', organization_id, 'owner_id', owner_id, 'name', name, 'template_id', template_id, 'image', image, 'access_mode', access_mode, 'state', state, 'cpu_millis', cpu_millis, 'memory_mib', memory_mib, 'gpu_count', gpu_count, 'disk_gib', disk_gib, 'generation', generation, 'created_at', created_at, 'updated_at', updated_at, 'deleted_at', deleted_at, 'runtime_profile', runtime_profile, 'template_snapshot_yaml', template_snapshot_yaml) item FROM workspaces WHERE installation_id = ?1 ORDER BY id",
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
    use super::normalize_snapshot_rows;

    #[test]
    fn old_catalog_rows_receive_template_yaml() {
        for table in ["workspace_templates", "workspaces"] {
            let rows = vec![serde_json::json!({
                "id": "legacy", "name": "Legacy", "image": "registry.example/dev:latest",
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
        }
    }
}
