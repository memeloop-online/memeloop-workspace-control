use std::collections::BTreeMap;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};

use super::{DatabaseSnapshot, StorageError, prepare_import};

#[test]
fn import_validates_asset_digest_before_decoding_postgres_rows() {
    let content = b"snapshot asset";
    let digest = format!("{:x}", Sha256::digest(content));
    let manifest = serde_json::json!({
        "id": "snapshot-plugin",
        "name": "Snapshot plugin",
        "version": "1.0.0",
        "description": "fixture",
        "wit_version": "0.2.0",
        "wasm": null,
        "workspace_create_policy": false,
        "denial_codes": [],
        "configuration": null,
        "assets": [{
            "path": "fixture.txt",
            "media_type": "application/json",
            "sha256": digest,
            "size_bytes": content.len()
        }],
        "ui_surfaces": [],
        "api_routes": [],
        "api_middleware": []
    })
    .to_string();
    let mut snapshot = DatabaseSnapshot {
        format_version: 1,
        schema_version: 14,
        installation_id: "snapshot-test".to_owned(),
        exported_at: 200,
        tables: BTreeMap::from([
            (
                "plugin_packages".to_owned(),
                vec![serde_json::json!({
                    "installation_id": "snapshot-test",
                    "plugin_id": "snapshot-plugin",
                    "manifest_json": manifest,
                    "component_bytes_base64": null,
                    "package_digest": "a".repeat(64),
                    "source_kind": "url",
                    "source_ref": "https://plugins.example/plugin.mwcpkg",
                    "source_confirmation": "administrator_confirmed",
                    "enabled": 1,
                    "approved_contributions_json": "[]",
                    "version": 1,
                    "created_by": uuid::Uuid::now_v7().to_string(),
                    "created_at": 100,
                    "updated_at": 100
                })],
            ),
            (
                "plugin_assets".to_owned(),
                vec![serde_json::json!({
                    "installation_id": "snapshot-test",
                    "plugin_id": "snapshot-plugin",
                    "asset_path": "fixture.txt",
                    "media_type": "application/json",
                    "content_bytes_base64": STANDARD.encode(content),
                    "content_digest": "0".repeat(64)
                })],
            ),
            (
                "plugin_catalog_metadata".to_owned(),
                vec![serde_json::json!({
                    "installation_id": "snapshot-test",
                    "revision": 1
                })],
            ),
        ]),
    };
    assert!(matches!(
        prepare_import(&snapshot),
        Err(StorageError::InvalidPluginSnapshot)
    ));

    snapshot.tables.get_mut("plugin_assets").unwrap()[0]["content_digest"] = digest.into();
    let normalized = prepare_import(&snapshot).unwrap().unwrap();
    assert_eq!(
        normalized["plugin_assets"][0]["content_bytes"],
        "\\x736e617073686f74206173736574"
    );
}
