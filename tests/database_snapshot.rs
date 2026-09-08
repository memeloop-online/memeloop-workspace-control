use std::collections::BTreeMap;

use base64::{Engine, engine::general_purpose::STANDARD};
use memeloop_workspace_control::{
    config::InstallationId,
    crypto::EnvelopeCipher,
    injections::{InjectionItem, InjectionKind, InjectionScope, InjectionValue},
    quota::Resources,
    storage::{
        ConfirmPluginInstall, CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate,
        Database, DatabaseSnapshot, InjectionScopeRef, PluginAssetBlob, PluginConfigurationWrite,
        StorageError, StorePluginInspection,
    },
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::AccessMode,
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
        .create_user_with_initial_key(
            "Snapshot User",
            TOKEN,
            true,
            memeloop_workspace_control::auth::ApiKeyScope::initial_key_defaults(true),
            31_536_000,
            100,
        )
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
    let workspace = create_snapshot_workspace(&database, organization.id, user.user_id).await;
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
    assert_eq!(snapshot.format_version, 2);
    assert_eq!(snapshot.schema_version, 22);
    assert_eq!(snapshot.installation_id, "snapshot-test");
    assert_eq!(snapshot.tables["injection_items"].len(), 1);
    assert!(snapshot.tables.contains_key("workspace_injection_refs"));
    assert_eq!(snapshot.tables["plugin_configurations"].len(), 1);
    assert_eq!(snapshot.tables["plugin_packages"].len(), 1);
    assert_eq!(snapshot.tables["plugin_assets"].len(), 1);
    assert_eq!(snapshot.tables["plugin_catalog_metadata"].len(), 1);
    let workspace_row = &snapshot.tables["workspaces"][0];
    for removed_field in [
        "runtime_namespace_scope",
        "runtime_namespace",
        "runtime_naming_scheme",
        "runtime_resource_prefix",
        "runtime_route_key",
        "arbitrary_unrecognized_workspace_snapshot_field",
    ] {
        assert!(workspace_row.get(removed_field).is_none());
    }
    let route_key = format!("snapshot-test-{}", workspace.short_id);
    assert_eq!(
        database
            .get_workspace_by_route_key(&route_key)
            .await
            .unwrap()
            .runtime,
        workspace.runtime
    );
    assert!(matches!(
        database
            .get_workspace_by_route_key(&workspace.short_id)
            .await,
        Err(StorageError::WorkspaceNotFound)
    ));
    assert!(matches!(
        database
            .get_workspace_by_route_key(&format!("other-installation-{}", workspace.short_id))
            .await,
        Err(StorageError::WorkspaceNotFound)
    ));
    let asset = &snapshot.tables["plugin_assets"][0];
    assert!(asset.get("content_bytes").is_none());
    assert_eq!(
        STANDARD
            .decode(asset["content_bytes_base64"].as_str().unwrap())
            .unwrap(),
        snapshot_asset()
    );
    assert_eq!(snapshot.tables["user_api_keys"].len(), 1);
    assert!(snapshot.tables["user_api_keys"][0]["allowed_template_ids_json"].is_null());
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
        .create_user_with_initial_key(
            "Snapshot Plugin User",
            TOKEN,
            true,
            memeloop_workspace_control::auth::ApiKeyScope::initial_key_defaults(true),
            31_536_000,
            100,
        )
        .await
        .unwrap();
    let organization = source
        .create_organization(
            CreateOrganization {
                name: "Snapshot workspace org".to_owned(),
                owner_user_id: user.user_id,
            },
            101,
        )
        .await
        .unwrap();
    let workspace = create_snapshot_workspace(&source, organization.id, user.user_id).await;
    install_snapshot_plugin(&source, user.user_id).await;
    let snapshot = source.export_snapshot(200).await.unwrap();

    let target = Database::connect(scoped_url.as_str(), installation)
        .await
        .unwrap();
    target.migrate().await.unwrap();

    // Import is replacement-only. Even ephemeral state that is intentionally
    // reset by export proves the destination is not empty.
    let Database::Postgres { pool, .. } = &target else {
        unreachable!("the import target is PostgreSQL");
    };
    let existing_event_id = Uuid::now_v7().to_string();
    sqlx::query(
        "INSERT INTO events \
         (id, installation_id, organization_id, workspace_id, kind, payload_json, created_at) \
         VALUES ($1, 'snapshot-pg', $2, NULL, 'snapshot.fixture', '{}', 1)",
    )
    .bind(&existing_event_id)
    .bind(Uuid::now_v7().to_string())
    .execute(pool)
    .await
    .unwrap();
    assert!(matches!(
        target.import_snapshot(&snapshot).await,
        Err(StorageError::ImportDestinationNotEmpty)
    ));
    sqlx::query("DELETE FROM events WHERE id = $1")
        .bind(existing_event_id)
        .execute(pool)
        .await
        .unwrap();

    // Import validates the canonical short identity rather than accepting a
    // syntactically-valid snapshot row that could target another resource.
    // Every failure must roll the whole import transaction back.
    let mut invalid_snapshot = snapshot.clone();
    let invalid_workspace_row = invalid_snapshot.tables.get_mut("workspaces").unwrap()[0]
        .as_object_mut()
        .unwrap();
    invalid_workspace_row.insert(
        "short_id".to_owned(),
        serde_json::Value::String("0000000000000000".to_owned()),
    );
    assert!(matches!(
        target.import_snapshot(&invalid_snapshot).await,
        Err(StorageError::InvalidWorkspace)
    ));
    let Database::Postgres { pool, .. } = &target else {
        unreachable!("the import target is PostgreSQL");
    };
    let workspace_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workspaces")
        .fetch_one(pool)
        .await
        .unwrap();
    let organization_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM organizations")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(workspace_count, 0);
    assert_eq!(organization_count, 0);

    // A v2 snapshot is a single runtime model. Supplying a removed dual-model
    // field is untrusted input and must be rejected before INSERT.
    for removed_field in [
        "runtime_namespace_scope",
        "runtime_namespace",
        "runtime_naming_scheme",
        "runtime_resource_prefix",
        "runtime_route_key",
        "arbitrary_unrecognized_workspace_snapshot_field",
    ] {
        let mut rejected_runtime_snapshot = snapshot.clone();
        rejected_runtime_snapshot
            .tables
            .get_mut("workspaces")
            .unwrap()[0]
            .as_object_mut()
            .unwrap()
            .insert(
                removed_field.to_owned(),
                serde_json::Value::String("untrusted".to_owned()),
            );
        assert!(matches!(
            target.import_snapshot(&rejected_runtime_snapshot).await,
            Err(StorageError::InvalidWorkspace)
        ));
    }

    let mut foreign_row_snapshot = snapshot.clone();
    for row in foreign_row_snapshot.tables.get_mut("users").unwrap() {
        row.as_object_mut().unwrap().insert(
            "installation_id".to_owned(),
            serde_json::Value::String("other-installation".to_owned()),
        );
    }
    assert!(matches!(
        target.import_snapshot(&foreign_row_snapshot).await,
        Err(StorageError::SnapshotRowInstallationMismatch { .. })
    ));
    let Database::Postgres { pool, .. } = &target else {
        unreachable!("the import target is PostgreSQL");
    };
    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(user_count, 0);

    // Template validation is independent of workspace decoding: an unused,
    // malformed template must not be imported merely because no workspace
    // references it yet.
    let mut invalid_template_snapshot = snapshot.clone();
    let mut invalid_unused_template = invalid_template_snapshot
        .tables
        .get("workspace_templates")
        .unwrap()[0]
        .clone();
    invalid_unused_template["id"] = serde_json::Value::String(Uuid::now_v7().to_string());
    invalid_unused_template["name"] =
        serde_json::Value::String("Invalid unused template".to_owned());
    invalid_unused_template["template_yaml"] =
        serde_json::Value::String("not a workspace template".to_owned());
    invalid_template_snapshot
        .tables
        .get_mut("workspace_templates")
        .unwrap()
        .push(invalid_unused_template);
    let invalid_template_error = target
        .import_snapshot(&invalid_template_snapshot)
        .await
        .expect_err("a malformed unused template must reject the snapshot");
    assert!(
        matches!(invalid_template_error, StorageError::InvalidTemplate),
        "unexpected snapshot import error: {invalid_template_error:?}"
    );
    let Database::Postgres { pool, .. } = &target else {
        unreachable!("the import target is PostgreSQL");
    };
    let template_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workspace_templates")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(template_count, 0);
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
    assert_eq!(
        target
            .get_workspace_by_route_key(&format!("snapshot-pg-{}", workspace.short_id))
            .await
            .unwrap()
            .runtime,
        workspace.runtime
    );

    drop(target);
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&administration)
        .await
        .unwrap();
    administration.close().await;
}

async fn create_snapshot_workspace(
    database: &Database,
    organization_id: Uuid,
    user_id: Uuid,
) -> memeloop_workspace_control::workspaces::Workspace {
    const IMAGE: &str = "registry.example/snapshot-workspace:1";
    database
        .upsert_image_policy(IMAGE, true, 102)
        .await
        .unwrap();
    let resources = Resources {
        cpu_millis: 1_000,
        memory_mib: 2_048,
        gpu_count: 0,
        disk_gib: 20,
    };
    let yaml = WorkspaceTemplateDocument::new(
        "Snapshot workspace",
        WorkspaceTemplateSpec::standard(IMAGE, AccessMode::Internal, resources),
    )
    .to_yaml()
    .unwrap();
    let template = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(organization_id),
                yaml,
            },
            true,
            103,
        )
        .await
        .unwrap();
    database
        .create_workspace(
            CreateWorkspace {
                organization_id,
                owner_id: user_id,
                name: "snapshot-workspace".to_owned(),
                template_id: template.id,
                resources: None,
                organization_injection_refs: None,
                user_injection_refs: None,
            },
            true,
            user_id,
            104,
        )
        .await
        .unwrap()
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
