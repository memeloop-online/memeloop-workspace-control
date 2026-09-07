use super::{validate_snapshot_row_installations, validate_snapshot_workspace_rows};
use crate::{config::InstallationId, workspace_runtime::workspace_short_id_for};
use uuid::Uuid;

#[test]
fn runtime_placement_is_validated_before_import() {
    let installation: InstallationId = "snapshot-test".parse().unwrap();
    let id = Uuid::parse_str("018f0000-0000-7000-8000-000000000001").unwrap();
    let short_id = workspace_short_id_for(id);
    let rows = vec![serde_json::json!({
        "id": id, "installation_id": installation.as_str(), "short_id": short_id,
        "runtime_namespace_scope": "dedicated", "runtime_namespace": ""
    })];
    assert!(validate_snapshot_workspace_rows(&rows, &installation).is_err());
}

#[test]
fn imported_rows_must_belong_to_the_snapshot_installation() {
    let installation: InstallationId = "snapshot-test".parse().unwrap();
    let rows = vec![serde_json::json!({"installation_id": "other-installation"})];
    assert!(validate_snapshot_row_installations("users", &rows, &installation).is_err());
}
