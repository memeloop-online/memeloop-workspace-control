use std::collections::BTreeMap;

use base64::{Engine, engine::general_purpose::STANDARD};
use memeloop_workspace_control::{
    crypto::EnvelopeCipher,
    injections::{InjectionItem, InjectionKind, InjectionScope, InjectionValue},
    storage::{CreateOrganization, Database, InjectionScopeRef, PluginConfigurationWrite},
};

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

    let snapshot = database.export_snapshot(200).await.unwrap();
    assert_eq!(snapshot.format_version, 1);
    assert_eq!(snapshot.schema_version, 11);
    assert_eq!(snapshot.installation_id, "snapshot-test");
    assert_eq!(snapshot.tables["injection_items"].len(), 1);
    assert!(snapshot.tables.contains_key("workspace_injection_refs"));
    assert_eq!(snapshot.tables["plugin_configurations"].len(), 1);
    assert!(!snapshot.tables.contains_key("web_shell_tickets"));
    assert!(!snapshot.tables.contains_key("workspace_leases"));
    assert!(!snapshot.tables.contains_key("idempotency_keys"));
    let serialized = serde_json::to_string(&snapshot).unwrap();
    assert!(!serialized.contains(secret_plaintext));
    assert!(!serialized.contains("line one"));
    assert!(serialized.contains("ciphertext"));
}
