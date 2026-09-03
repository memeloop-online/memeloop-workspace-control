use std::collections::BTreeMap;

use base64::{Engine, engine::general_purpose::STANDARD};
use memeloop_workspace_control::{
    config::InstallationId,
    crypto::EnvelopeCipher,
    injections::{InjectionItem, InjectionKind, InjectionScope, InjectionValue},
    quota::Resources,
    storage::{
        CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate, Database,
        IdempotencyCompletion, IdempotencyDecision, InjectionScopeRef, StorageError,
    },
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::AccessMode,
};
use uuid::Uuid;

fn item(key: &str, target: &str, locked: bool) -> InjectionItem {
    InjectionItem {
        key: key.to_owned(),
        kind: InjectionKind::EnvironmentVariable,
        target: target.to_owned(),
        value: InjectionValue::Utf8("not-returned".to_owned()),
        sensitive: true,
        locked,
        version: 0,
        file_mode: None,
        owner: None,
        group: None,
        template_selector: None,
        labels: BTreeMap::new(),
    }
}

async fn assert_locked_batch_rolls_back(database: Database) {
    database.migrate().await.unwrap();
    let actor = database
        .create_user(
            "Batch Operator",
            "batch-operator-token-000000000000000000000",
            true,
            1,
        )
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Batch organization".to_owned(),
                owner_user_id: actor.user_id,
            },
            2,
        )
        .await
        .unwrap();
    let scope = InjectionScopeRef {
        scope: InjectionScope::Organization,
        scope_id: organization.id,
    };
    let cipher = EnvelopeCipher::from_base64(&STANDARD.encode([17_u8; 32])).unwrap();
    database
        .replace_injection(
            &cipher,
            scope,
            item("a-ordinary", "ORDINARY_VALUE", false),
            actor.user_id,
            3,
        )
        .await
        .unwrap();
    database
        .replace_injection(
            &cipher,
            scope,
            item("z-locked", "LOCKED_VALUE", true),
            actor.user_id,
            4,
        )
        .await
        .unwrap();

    database
        .upsert_image_policy("registry.example/workspace:1", true, 5)
        .await
        .unwrap();
    let template = WorkspaceTemplateDocument::new(
        "Batch template",
        WorkspaceTemplateSpec::standard(
            "registry.example/workspace:1",
            AccessMode::Internal,
            Resources {
                cpu_millis: 500,
                memory_mib: 512,
                gpu_count: 0,
                disk_gib: 5,
            },
        ),
    );
    let template_id = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(organization.id),
                yaml: template.to_yaml().unwrap(),
            },
            true,
            6,
        )
        .await
        .unwrap()
        .id;
    database
        .create_workspace(
            CreateWorkspace {
                organization_id: organization.id,
                owner_id: actor.user_id,
                name: "batch-workspace".to_owned(),
                template_id,
                resources: None,
                organization_injection_refs: None,
                user_injection_refs: None,
            },
            true,
            actor.user_id,
            7,
        )
        .await
        .unwrap();
    let before = database.job_counts().await.unwrap().pending;

    let missing_reservation = database
        .delete_injections_and_enqueue_reconciles(
            scope,
            &["z-locked".to_owned(), "a-ordinary".to_owned()],
            true,
            actor.user_id,
            8,
            IdempotencyCompletion {
                scope: "missing-reservation",
                key: "missing-reservation-key",
                request_hash: "missing-reservation-hash",
                status_code: 204,
                response_json: "",
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        missing_reservation,
        StorageError::IdempotencyReservationLost
    ));
    assert_eq!(
        database
            .list_injection_summaries(scope)
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(database.job_counts().await.unwrap().pending, before);

    assert_eq!(
        database
            .begin_idempotency("locked-batch", "locked-key", "locked-hash", 9, 109)
            .await
            .unwrap(),
        IdempotencyDecision::Reserved
    );
    let error = database
        .delete_injections_and_enqueue_reconciles(
            scope,
            &["z-locked".to_owned(), "a-ordinary".to_owned()],
            false,
            actor.user_id,
            9,
            IdempotencyCompletion {
                scope: "locked-batch",
                key: "locked-key",
                request_hash: "locked-hash",
                status_code: 204,
                response_json: "",
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, StorageError::InvalidInjectionLock));
    assert_eq!(
        database
            .list_injection_summaries(scope)
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(database.job_counts().await.unwrap().pending, before);
    database
        .abandon_idempotency("locked-batch", "locked-key", "locked-hash")
        .await
        .unwrap();

    assert_eq!(
        database
            .begin_idempotency("successful-batch", "success-key", "success-hash", 10, 110)
            .await
            .unwrap(),
        IdempotencyDecision::Reserved
    );
    let deleted = database
        .delete_injections_and_enqueue_reconciles(
            scope,
            &["z-locked".to_owned(), "a-ordinary".to_owned()],
            true,
            actor.user_id,
            10,
            IdempotencyCompletion {
                scope: "successful-batch",
                key: "success-key",
                request_hash: "success-hash",
                status_code: 204,
                response_json: "",
            },
        )
        .await
        .unwrap();
    assert_eq!(deleted, 2);
    assert_eq!(database.job_counts().await.unwrap().pending, before + 1);
    assert!(matches!(
        database
            .begin_idempotency("successful-batch", "success-key", "success-hash", 11, 111)
            .await
            .unwrap(),
        IdempotencyDecision::Replay(_)
    ));

    assert_eq!(
        database
            .begin_idempotency("missing-batch", "missing-key", "missing-hash", 12, 112)
            .await
            .unwrap(),
        IdempotencyDecision::Reserved
    );
    let repeated = database
        .delete_injections_and_enqueue_reconciles(
            scope,
            &["a-ordinary".to_owned(), "z-locked".to_owned()],
            true,
            actor.user_id,
            12,
            IdempotencyCompletion {
                scope: "missing-batch",
                key: "missing-key",
                request_hash: "missing-hash",
                status_code: 204,
                response_json: "",
            },
        )
        .await
        .unwrap();
    assert_eq!(repeated, 0);
    assert_eq!(database.job_counts().await.unwrap().pending, before + 1);
}

#[tokio::test]
async fn sqlite_batch_delete_is_atomic() {
    let database = Database::connect(
        "sqlite::memory:",
        "batch-delete-sqlite".parse::<InstallationId>().unwrap(),
    )
    .await
    .unwrap();
    assert_locked_batch_rolls_back(database).await;
}

#[tokio::test]
async fn postgres_batch_delete_is_atomic() {
    let Ok(database_url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        eprintln!("skipping PostgreSQL injection batch test: MWC_TEST_POSTGRES_URL is not set");
        return;
    };
    let schema = format!("mwc_injection_batch_{}", Uuid::now_v7().simple());
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
    let database = Database::connect(scoped_url.as_str(), "batch-pg".parse().unwrap())
        .await
        .unwrap();
    assert_locked_batch_rolls_back(database).await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&administration)
        .await
        .unwrap();
    administration.close().await;
}
