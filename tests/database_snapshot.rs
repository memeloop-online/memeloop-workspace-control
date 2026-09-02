use std::collections::BTreeMap;

use base64::{Engine, engine::general_purpose::STANDARD};
use memeloop_workspace_control::{
    config::InstallationId,
    crypto::EnvelopeCipher,
    injections::{InjectionItem, InjectionKind, InjectionScope, InjectionValue},
    storage::{
        ConfirmPluginInstall, CreateOrganization, Database, DatabaseSnapshot, InjectionScopeRef,
        PluginAssetBlob, PluginConfigurationWrite, StorePluginInspection,
    },
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const TOKEN: &str = "snapshot-user-token-00000000000000000000000000";

#[tokio::test]
async fn sqlite_snapshot_contains_ciphertext_and_resets_only_pending_work() {
    let database = Database::connect("sqlite::memory:", "snapshot-test".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let user = database
        .create_user("Snapshot User", TOKEN, true, 100)
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Snapshot Org".to_owned(),
                owner_user_id: user.user_id,
            },
            101,
        )
        .await
        .unwrap();
    let cipher = EnvelopeCipher::from_base64(&STANDARD.encode([4_u8; 32])).unwrap();
    let secret_plaintext = "line one\nline two\n";
    database
        .replace_injection(
            &cipher,
            InjectionScopeRef {
                scope: InjectionScope::Organization,
                scope_id: organization.id,
            },
            InjectionItem {
                key: "private-config".to_owned(),
                kind: InjectionKind::SecretFile,
                target: "/run/private/config".to_owned(),
                value: InjectionValue::Utf8(secret_plaintext.to_owned()),
                sensitive: true,
                locked: false,
                version: 0,
                file_mode: Some(0o600),
                owner: None,
                group: None,
                template_selector: None,
                labels: BTreeMap::new(),
            },
            user.user_id,
            102,
        )
        .await
        .unwrap();

    let schema_digest = "a".repeat(64);
    database
        .put_plugin_configuration(PluginConfigurationWrite {
            plugin_id: "snapshot-policy",
            organization_id: Some(organization.id),
            value: &serde_json::json!({"maximum": 3}),
            schema_digest: &schema_digest,
            expected_version: 0,
            actor_user_id: user.user_id,
            now: 103,
        })
        .await
        .unwrap();
    install_snapshot_plugin(&database, user.user_id).await;

    let snapshot = database.export_snapshot(200).await.unwrap();
    assert_eq!(snapshot.format_version, 1);
    assert_eq!(snapshot.schema_version, 16);
    assert_eq!(snapshot.installation_id, "snapshot-test");
    assert_eq!(snapshot.tables["injection_items"].len(), 1);
    assert!(snapshot.tables.contains_key("workspace_injection_refs"));
    assert_eq!(snapshot.tables["plugin_configurations"].len(), 1);
    assert_eq!(snapshot.tables["plugin_packages"].len(), 1);
    assert_eq!(snapshot.tables["plugin_assets"].len(), 1);
    assert_eq!(snapshot.tables["plugin_catalog_metadata"].len(), 1);
    let asset = &snapshot.tables["plugin_assets"][0];
    assert!(asset.get("content_bytes").is_none());
    assert_eq!(
        STANDARD
            .decode(asset["content_bytes_base64"].as_str().unwrap())
            .unwrap(),
        snapshot_asset()
    );
    assert_eq!(snapshot.tables["user_api_keys"].len(), 1);
    assert!(!snapshot.tables.contains_key("web_shell_tickets"));
    assert!(!snapshot.tables.contains_key("workspace_leases"));
    assert!(!snapshot.tables.contains_key("idempotency_keys"));
    let serialized = serde_json::to_string(&snapshot).unwrap();
    let roundtrip: DatabaseSnapshot = serde_json::from_str(&serialized).unwrap();
    assert_eq!(roundtrip.tables["plugin_packages"][0]["enabled"], 1);
    assert_eq!(roundtrip.tables["plugin_packages"][0]["version"], 1);
    assert_eq!(
        roundtrip.tables["plugin_packages"][0]["source_confirmation"],
        "administrator_confirmed"
    );
    assert_eq!(
        roundtrip.tables["plugin_packages"][0]["source_ref"],
        "https://plugins.example/snapshot-plugin.mwcpkg"
    );
    assert!(!serialized.contains(secret_plaintext));
    assert!(!serialized.contains("line one"));
    assert!(!serialized.contains("snapshot-secret-token"));
    assert!(!serialized.contains(std::str::from_utf8(&snapshot_asset()).unwrap()));
    assert!(serialized.contains("ciphertext"));
}

#[tokio::test]
async fn postgres_import_restores_dynamic_plugin_package_and_assets_when_configured() {
    let Ok(database_url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        eprintln!("skipping PostgreSQL snapshot test: MWC_TEST_POSTGRES_URL is not set");
        return;
    };
    let schema = format!("mwc_snapshot_{}", Uuid::now_v7().simple());
    let administration = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .unwrap();
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&administration)
        .await
        .unwrap();
    let mut scoped_url = url::Url::parse(&database_url).unwrap();
    scoped_url
        .query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema}"));
    let installation: InstallationId = "snapshot-pg".parse().unwrap();
    let source = Database::connect("sqlite::memory:", installation.clone())
        .await
        .unwrap();
    source.migrate().await.unwrap();
    let user = source
        .create_user("Snapshot Plugin User", TOKEN, true, 100)
        .await
        .unwrap();
    install_snapshot_plugin(&source, user.user_id).await;
    let snapshot = source.export_snapshot(200).await.unwrap();

    let target = Database::connect(scoped_url.as_str(), installation)
        .await
        .unwrap();
    target.migrate().await.unwrap();
    target.import_snapshot(&snapshot).await.unwrap();
    let packages = target.list_plugin_packages().await.unwrap();
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].plugin_id, "snapshot-plugin");
    assert!(packages[0].enabled);
    assert_eq!(packages[0].version, 1);
    assert_eq!(
        packages[0].approved_contributions,
        ["configuration", "ui_surfaces"]
    );
    assert_eq!(packages[0].source_kind, "url");
    assert_eq!(
        packages[0].source_ref,
        "https://plugins.example/snapshot-plugin.mwcpkg"
    );
    let assets = target.plugin_assets("snapshot-plugin").await.unwrap();
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0].content, snapshot_asset());
    assert!(target.plugin_catalog_revision().await.unwrap() >= 1);

    drop(target);
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&administration)
        .await
        .unwrap();
    administration.close().await;
}

async fn install_snapshot_plugin(database: &Database, user_id: Uuid) {
    let asset = snapshot_asset();
    let asset_digest = format!("{:x}", Sha256::digest(&asset));
    let manifest = serde_json::json!({
        "id": "snapshot-plugin",
        "name": "Snapshot plugin",
        "version": "1.2.3",
        "description": "Snapshot transfer fixture",
        "wit_version": "0.2.0",
        "wasm": null,
        "workspace_create_policy": false,
        "denial_codes": [],
        "configuration": {
            "schema": {
                "type": "object",
                "properties": {"label": {"type": "string"}},
                "additionalProperties": false
            },
            "default": {"label": "snapshot"}
        },
        "assets": [{
            "path": "index.html",
            "media_type": "text/html",
            "sha256": asset_digest,
            "size_bytes": asset.len()
        }],
        "ui_surfaces": [{
            "id": "snapshot-panel",
            "title": "Snapshot panel",
            "placement": "admin_tab",
            "entrypoint": "index.html",
            "allowed_bridge_methods": ["theme.read"]
        }],
        "api_routes": [],
        "api_middleware": []
    })
    .to_string();
    let package_digest = format!("{:x}", Sha256::digest(b"snapshot-plugin-package"));
    let contributions = vec!["configuration".to_owned(), "ui_surfaces".to_owned()];
    let inspection = database
        .store_plugin_inspection(StorePluginInspection {
            plugin_id: "snapshot-plugin".to_owned(),
            manifest_json: manifest,
            component_bytes: None,
            package_digest: package_digest.clone(),
            size_bytes: asset.len() as u64,
            source_kind: "url".to_owned(),
            source_ref:
                "https://plugins.example/snapshot-plugin.mwcpkg?token=snapshot-secret-token"
                    .to_owned(),
            source_confirmation: "administrator_confirmed".to_owned(),
            declared_contributions: contributions.clone(),
            assets: vec![PluginAssetBlob {
                path: "index.html".to_owned(),
                media_type: "text/html".to_owned(),
                content: asset,
                digest: asset_digest,
            }],
            created_by: user_id,
            now: 104,
            expires_at: 1_004,
        })
        .await
        .unwrap();
    assert_eq!(
        inspection.source_ref,
        "https://plugins.example/snapshot-plugin.mwcpkg"
    );
    database
        .confirm_plugin_install(ConfirmPluginInstall {
            inspection_id: inspection.id,
            expected_digest: &package_digest,
            expected_package_version: 0,
            approved_contributions: &contributions,
            enabled: true,
            actor_user_id: user_id,
            now: 105,
        })
        .await
        .unwrap();
}

fn snapshot_asset() -> Vec<u8> {
    b"<!doctype html><title>Snapshot plugin</title>".to_vec()
}
