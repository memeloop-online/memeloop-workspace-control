use std::collections::BTreeMap;

use memeloop_workspace_control::{
    crypto::EnvelopeCipher,
    injections::{InjectionItem, InjectionKind, InjectionScope, InjectionValue},
    quota::Resources,
    storage::{
        CreateOrganization, CreateWebhookSubscription, CreateWorkspace, CreateWorkspaceTemplate,
        Database, InjectionScopeRef, StorageError,
    },
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::{AccessMode, Workspace, WorkspaceAction, WorkspaceObservation, WorkspaceState},
};
use serde_json::{Value, json};
use uuid::Uuid;

async fn retained_state(database: &Database, workspace: &Workspace) -> Value {
    let mut counts = BTreeMap::<&str, i64>::new();
    for table in ["web_shell_tickets", "workspace_tombstones"] {
        let sql = format!(
            "SELECT COUNT(*) FROM {table} WHERE installation_id = $1 AND workspace_id = $2"
        );
        let count = match database {
            Database::Sqlite { pool, .. } => sqlx::query_scalar(&sql)
                .bind(database.installation_id().as_str())
                .bind(workspace.id.to_string())
                .fetch_one(pool)
                .await
                .unwrap(),
            Database::Postgres { pool, .. } => sqlx::query_scalar(&sql)
                .bind(database.installation_id().as_str())
                .bind(workspace.id.to_string())
                .fetch_one(pool)
                .await
                .unwrap(),
        };
        counts.insert(table, count);
    }
    let stored_workspace = match database.get_workspace(workspace.id).await {
        Ok(workspace) => Some(workspace),
        Err(StorageError::WorkspaceNotFound) => None,
        Err(error) => panic!("workspace lookup failed: {error}"),
    };
    json!({
        "workspace": stored_workspace,
        "audit": database.list_audit(workspace.organization_id, 100).await.unwrap(),
        "events": database.list_events(workspace.organization_id, None, 100).await.unwrap(),
        "jobs": database.job_counts().await.unwrap(),
        "ssh": database.workspace_ssh_public_identity(workspace.id).await.unwrap(),
        "injections": database.list_injection_summaries(InjectionScopeRef {
            scope: InjectionScope::Workspace,
            scope_id: workspace.id,
        }).await.unwrap(),
        "counts": counts,
    })
}

async fn assert_stale_is_ignored(
    writer: &Database,
    observer: &Database,
    snapshot: &Workspace,
    observation: WorkspaceObservation,
) {
    let before = retained_state(writer, snapshot).await;
    assert!(
        observer
            .record_workspace_observation(
                snapshot.id,
                snapshot.generation,
                observation,
                snapshot.owner_id,
                100,
            )
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(retained_state(writer, snapshot).await, before);
}

async fn observe(
    observer: &Database,
    workspace: &Workspace,
    observation: WorkspaceObservation,
    now: i64,
) -> Workspace {
    observer
        .record_workspace_observation(
            workspace.id,
            workspace.generation,
            observation,
            workspace.owner_id,
            now,
        )
        .await
        .unwrap()
        .expect("the current generation must accept its observation")
}

async fn exercise_interleaved_observations(writer: &Database, observer: &Database) {
    writer.migrate().await.unwrap();
    writer
        .upsert_image_policy("registry.example/workspace:1", true, 1)
        .await
        .unwrap();
    let admin = writer
        .create_user_with_initial_key(
            "Observation administrator",
            "observation-admin-0000000000000000000000000000",
            true,
            memeloop_workspace_control::auth::ApiKeyScope::initial_key_defaults(true),
            31_536_000,
            1,
        )
        .await
        .unwrap();
    let organization = writer
        .create_organization(
            CreateOrganization {
                name: "Observation generation".to_owned(),
                owner_user_id: admin.user_id,
            },
            2,
        )
        .await
        .unwrap();
    let cipher =
        EnvelopeCipher::from_base64(&EnvelopeCipher::generate_base64_key().unwrap()).unwrap();
    writer
        .create_webhook_subscription(
            &cipher,
            CreateWebhookSubscription {
                organization_id: organization.id,
                url: "https://hooks.example.com/workspaces".to_owned(),
                event_prefix: "workspace.".to_owned(),
                signing_secret: "generation-observation-test-signing-secret".to_owned(),
            },
            admin.user_id,
            3,
        )
        .await
        .unwrap();
    let template = writer
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(organization.id),
                yaml: WorkspaceTemplateDocument::new(
                    "Observation workspace",
                    WorkspaceTemplateSpec::standard(
                        "registry.example/workspace:1",
                        AccessMode::Internal,
                        Resources {
                            cpu_millis: 500,
                            memory_mib: 512,
                            gpu_count: 0,
                            disk_gib: 10,
                        },
                    ),
                )
                .to_yaml()
                .unwrap(),
            },
            true,
            3,
        )
        .await
        .unwrap();
    let created = writer
        .create_workspace(
            CreateWorkspace {
                organization_id: organization.id,
                owner_id: admin.user_id,
                name: "interleaved-observations".to_owned(),
                template_id: template.id,
                resources: None,
                temporary_storage_gib: None,
                organization_injection_refs: None,
                user_injection_refs: None,
            },
            true,
            admin.user_id,
            4,
        )
        .await
        .unwrap();

    // Kubernetes confirmed the old Ready snapshot before another request accepted restart.
    let old_ready = observer.get_workspace(created.id).await.unwrap();
    let restarting = writer
        .request_workspace_action(created.id, WorkspaceAction::Restart, admin.user_id, 5)
        .await
        .unwrap();
    assert_stale_is_ignored(writer, observer, &old_ready, WorkspaceObservation::Ready).await;
    assert_stale_is_ignored(writer, observer, &old_ready, WorkspaceObservation::NotReady).await;
    assert_eq!(
        writer.get_workspace(created.id).await.unwrap().state,
        WorkspaceState::Restarting
    );
    let ready = observe(observer, &restarting, WorkspaceObservation::Ready, 6).await;
    assert_eq!(ready.state, WorkspaceState::Ready);

    let stopping = writer
        .request_workspace_action(ready.id, WorkspaceAction::Stop, admin.user_id, 7)
        .await
        .unwrap();
    let old_stopped = observer.get_workspace(stopping.id).await.unwrap();
    let stopped = observe(observer, &stopping, WorkspaceObservation::Stopped, 8).await;
    let starting = writer
        .request_workspace_action(stopped.id, WorkspaceAction::Start, admin.user_id, 9)
        .await
        .unwrap();
    assert_stale_is_ignored(
        writer,
        observer,
        &old_stopped,
        WorkspaceObservation::Stopped,
    )
    .await;
    assert_eq!(
        writer.get_workspace(starting.id).await.unwrap().state,
        WorkspaceState::Starting
    );
    // The same old completion must not satisfy a later stop either, even though state matches.
    let stopping_again = writer
        .request_workspace_action(starting.id, WorkspaceAction::Stop, admin.user_id, 10)
        .await
        .unwrap();
    assert_stale_is_ignored(
        writer,
        observer,
        &old_stopped,
        WorkspaceObservation::Stopped,
    )
    .await;
    let stopped = observe(observer, &stopping_again, WorkspaceObservation::Stopped, 11).await;
    let starting = writer
        .request_workspace_action(stopped.id, WorkspaceAction::Start, admin.user_id, 12)
        .await
        .unwrap();
    let ready = observe(observer, &starting, WorkspaceObservation::Ready, 13).await;

    writer
        .ensure_workspace_ssh_identity(&cipher, ready.id, 14)
        .await
        .unwrap();
    writer
        .replace_injection(
            &cipher,
            InjectionScopeRef {
                scope: InjectionScope::Workspace,
                scope_id: ready.id,
            },
            InjectionItem {
                key: "generation-secret".to_owned(),
                kind: InjectionKind::SecretFile,
                target: "/home/workspace/generation-secret".to_owned(),
                value: InjectionValue::Utf8("retain-until-confirmed-deletion".to_owned()),
                sensitive: true,
                locked: false,
                version: 0,
                file_mode: Some(0o600),
                owner: Some("workspace".to_owned()),
                group: Some("workspace".to_owned()),
                template_selector: None,
                labels: BTreeMap::new(),
            },
            admin.user_id,
            14,
        )
        .await
        .unwrap();
    let ticket = writer
        .issue_web_shell_ticket(organization.id, ready.id, admin.user_id, 14, 300)
        .await
        .unwrap();
    let deleting = writer
        .request_workspace_action(ready.id, WorkspaceAction::Delete, admin.user_id, 15)
        .await
        .unwrap();
    let old_deleted = observer.get_workspace(deleting.id).await.unwrap();
    let failed = observe(observer, &deleting, WorkspaceObservation::Failed, 16).await;
    let deleting_again = writer
        .request_workspace_action(failed.id, WorkspaceAction::Delete, admin.user_id, 17)
        .await
        .unwrap();
    assert_stale_is_ignored(
        writer,
        observer,
        &old_deleted,
        WorkspaceObservation::Deleted,
    )
    .await;
    let retained = retained_state(writer, &deleting_again).await;
    assert_eq!(retained["workspace"]["state"], "deleting");
    assert!(!retained["ssh"].is_null());
    assert_eq!(retained["injections"].as_array().unwrap().len(), 1);
    assert_eq!(retained["counts"]["web_shell_tickets"], 1);
    assert_eq!(retained["counts"]["workspace_tombstones"], 0);

    // A current-generation deletion still scrubs credentials and leaves its tombstone.
    let deleted = observe(observer, &deleting_again, WorkspaceObservation::Deleted, 18).await;
    assert_eq!(deleted.state, WorkspaceState::Deleted);
    let scrubbed = retained_state(writer, &deleted).await;
    assert!(scrubbed["workspace"].is_null());
    assert!(scrubbed["ssh"].is_null());
    assert!(scrubbed["injections"].as_array().unwrap().is_empty());
    assert_eq!(scrubbed["counts"]["web_shell_tickets"], 0);
    assert_eq!(scrubbed["counts"]["workspace_tombstones"], 1);
    assert!(
        writer
            .consume_web_shell_ticket(&ticket.ticket, ready.id, 19)
            .await
            .unwrap()
            .is_none()
    );
    assert_stale_is_ignored(
        writer,
        observer,
        &old_deleted,
        WorkspaceObservation::Deleted,
    )
    .await;
}

#[tokio::test]
async fn sqlite_observations_do_not_finish_or_scrub_a_newer_generation() {
    let database = Database::connect("sqlite::memory:", "observation-ci".parse().unwrap())
        .await
        .unwrap();
    exercise_interleaved_observations(&database, &database).await;
}

#[tokio::test]
async fn postgres_replicas_ignore_superseded_observations() {
    let Ok(database_url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        eprintln!("skipping PostgreSQL observation test: MWC_TEST_POSTGRES_URL is not set");
        return;
    };
    let schema = format!("mwc_observation_{}", Uuid::now_v7().simple());
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
    let writer = Database::connect(scoped_url.as_str(), "observation-ci".parse().unwrap())
        .await
        .unwrap();
    let observer = Database::connect(scoped_url.as_str(), "observation-ci".parse().unwrap())
        .await
        .unwrap();
    exercise_interleaved_observations(&writer, &observer).await;
    drop(writer);
    drop(observer);
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&administration)
        .await
        .unwrap();
}
