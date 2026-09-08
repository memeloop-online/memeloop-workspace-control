use serde_json::{Map, Value};
use uuid::Uuid;

use crate::{
    config::InstallationId,
    storage::StorageError,
    workspace_runtime::{WorkspaceRuntimeIdentity, workspace_short_id_for},
};

// This is the complete schema-v22 `workspaces` column set, matching the export query.
// PostgreSQL's json_populate_recordset ignores unknown JSON members, so accepting a subset
// here would make an untrusted snapshot differ from the data that is actually imported.
const WORKSPACE_COLUMNS: &[&str] = &[
    "id",
    "installation_id",
    "short_id",
    "organization_id",
    "owner_id",
    "name",
    "template_id",
    "image",
    "access_mode",
    "state",
    "cpu_millis",
    "memory_mib",
    "gpu_count",
    "disk_gib",
    "generation",
    "created_at",
    "updated_at",
    "deleted_at",
    "template_snapshot_yaml",
];

pub(super) fn validate(
    rows: &[Value],
    installation_id: &InstallationId,
) -> Result<(), StorageError> {
    for row in rows {
        let object = row.as_object().ok_or(StorageError::InvalidWorkspace)?;
        validate_columns(object)?;
        let id = Uuid::parse_str(&required_string(object, "id")?)
            .map_err(|_| StorageError::InvalidWorkspace)?;
        let short_id = required_string(object, "short_id")?;
        if short_id != workspace_short_id_for(id) {
            return Err(StorageError::InvalidWorkspace);
        }
        WorkspaceRuntimeIdentity
            .validate_for_workspace(installation_id, id, &short_id)
            .map_err(|_| StorageError::InvalidWorkspace)?;
    }
    Ok(())
}

fn validate_columns(object: &Map<String, Value>) -> Result<(), StorageError> {
    if object.len() != WORKSPACE_COLUMNS.len()
        || WORKSPACE_COLUMNS
            .iter()
            .any(|column| !object.contains_key(*column))
    {
        return Err(StorageError::InvalidWorkspace);
    }
    Ok(())
}

fn required_string(object: &Map<String, Value>, key: &str) -> Result<String, StorageError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(StorageError::InvalidWorkspace)
}
