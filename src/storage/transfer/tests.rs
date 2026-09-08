use super::{validate_snapshot_row_installations, workspace_rows::validate};
use crate::{config::InstallationId, workspace_runtime::workspace_short_id_for};
use uuid::Uuid;

#[test]
fn workspace_short_identity_is_validated_before_import() {
    let installation: InstallationId = "snapshot-test".parse().unwrap();
    let id = Uuid::parse_str("018f0000-0000-7000-8000-000000000001").unwrap();
    let short_id = workspace_short_id_for(id);
    let mut rows = vec![serde_json::json!({
        "id": id, "installation_id": installation.as_str(), "short_id": short_id,
        "organization_id": id, "owner_id": id, "name": "test", "template_id": id,
        "image": "example/dev:1", "access_mode": "internal", "state": "stopped",
        "cpu_millis": 1000, "memory_mib": 1024, "gpu_count": 0, "disk_gib": 10,
        "generation": 1, "created_at": 1, "updated_at": 1, "deleted_at": null,
        "template_snapshot_yaml": ""
    })];
    assert!(validate(&rows, &installation).is_ok());
    rows[0]["short_id"] = serde_json::json!(format!("{short_id}0"));
    assert!(validate(&rows, &installation).is_err());
}

#[test]
fn imported_rows_must_belong_to_the_snapshot_installation() {
    let installation: InstallationId = "snapshot-test".parse().unwrap();
    let rows = vec![serde_json::json!({"installation_id": "other-installation"})];
    assert!(validate_snapshot_row_installations("users", &rows, &installation).is_err());
}
