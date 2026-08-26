use std::collections::BTreeMap;

use base64::{Engine, engine::general_purpose::STANDARD};
use memeloop_workspace_control::{
    crypto::EnvelopeCipher,
    injections::{
        InjectionItem, InjectionKind, InjectionScope, InjectionValue, resolve_injections,
    },
    storage::{Database, InjectionScopeRef, StorageError},
};
use uuid::Uuid;

fn cipher(byte: u8) -> EnvelopeCipher {
    EnvelopeCipher::from_base64(&STANDARD.encode([byte; 32])).unwrap()
}

fn item(key: &str, value: InjectionValue, locked: bool) -> InjectionItem {
    InjectionItem {
        key: key.to_owned(),
        kind: InjectionKind::SecretFile,
        target: format!("/workspace/.secrets/{key}"),
        value,
        sensitive: true,
        locked,
        version: 0,
        file_mode: Some(0o600),
        owner: Some("workspace".to_owned()),
        group: Some("workspace".to_owned()),
        template_selector: Some("rust".to_owned()),
        labels: BTreeMap::from([("environment".to_owned(), "test".to_owned())]),
    }
}

#[tokio::test]
async fn encrypted_injections_are_versioned_write_only_and_round_trip_exactly() {
    let database = Database::connect("sqlite::memory:", "injection-test".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let scope = InjectionScopeRef {
        scope: InjectionScope::Organization,
        scope_id: Uuid::now_v7(),
    };
    let actor = Uuid::now_v7();
    let multiline = "first\n\n  indented\nlast\n";
    let first = database
        .replace_injection(
            &cipher(7),
            scope,
            item(
                "registry-token",
                InjectionValue::Utf8(multiline.to_owned()),
                true,
            ),
            actor,
            10,
        )
        .await
        .unwrap();
    assert_eq!(first.version, 1);
    let summary_json = serde_json::to_string(&first).unwrap();
    assert!(!summary_json.contains(multiline));

    let loaded = database.load_injections(&cipher(7), scope).await.unwrap();
    assert_eq!(loaded[0].value, InjectionValue::Utf8(multiline.to_owned()));
    assert_eq!(loaded[0].file_mode, Some(0o600));

    let second = database
        .replace_injection(
            &cipher(7),
            scope,
            item(
                "registry-token",
                InjectionValue::Utf8("replacement\n".to_owned()),
                true,
            ),
            actor,
            11,
        )
        .await
        .unwrap();
    assert_eq!(second.version, 2);
    let summaries = database.list_injection_summaries(scope).await.unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].version, 2);
    assert!(database.load_injections(&cipher(8), scope).await.is_err());
}

#[tokio::test]
async fn binary_values_and_locked_cascade_are_preserved() {
    let database = Database::connect("sqlite::memory:", "injection-test".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let organization_scope = InjectionScopeRef {
        scope: InjectionScope::Organization,
        scope_id: Uuid::now_v7(),
    };
    let bytes = [0_u8, 255, 1, 2, 10];
    database
        .replace_injection(
            &cipher(3),
            organization_scope,
            item(
                "binary",
                InjectionValue::Base64(STANDARD.encode(bytes)),
                true,
            ),
            Uuid::now_v7(),
            1,
        )
        .await
        .unwrap();
    let organization = database
        .load_injections(&cipher(3), organization_scope)
        .await
        .unwrap();
    assert_eq!(
        organization[0].value,
        InjectionValue::Base64(STANDARD.encode(bytes))
    );
    let override_item = item(
        "binary",
        InjectionValue::Base64(STANDARD.encode([9_u8])),
        false,
    );
    assert!(resolve_injections(&organization, &[override_item], &[]).is_err());
}

#[tokio::test]
async fn user_and_workspace_scopes_cannot_set_locked_flag() {
    let database = Database::connect("sqlite::memory:", "injection-test".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let result = database
        .replace_injection(
            &cipher(1),
            InjectionScopeRef {
                scope: InjectionScope::User,
                scope_id: Uuid::now_v7(),
            },
            item(
                "invalid-lock",
                InjectionValue::Utf8("value".to_owned()),
                true,
            ),
            Uuid::now_v7(),
            1,
        )
        .await;
    assert!(matches!(result, Err(StorageError::InvalidInjectionLock)));
}
