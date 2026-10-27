use memeloop_workspace_control::{
    quota::Resources,
    storage::{
        CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate, Database, StorageError,
    },
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::{AccessMode, WorkspaceAction, WorkspaceObservation},
};
use uuid::Uuid;

const OLD_IMAGE: &str = "registry.example/workspace@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[tokio::test]
async fn postgres_updates_a_locked_stopped_workspace_cpu_overcommit() {
    let Ok(database_url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        eprintln!("skipping PostgreSQL CPU overcommit test: MWC_TEST_POSTGRES_URL is not set");
        return;
    };
    let schema = format!("mwc_cpu_overcommit_{}", Uuid::now_v7().simple());
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
    let suffix = &Uuid::now_v7().simple().to_string()[..11];
    let installation_id = format!("cpu-pg-{suffix}").parse().unwrap();
    let database = Database::connect(scoped_url.as_str(), installation_id)
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
        .upsert_image_policy(OLD_IMAGE, true, 1)
        .await
        .unwrap();
    let admin = database
        .create_user_with_initial_key(
            "Admin",
            "cpu-overcommit-pg-admin-000000000000000000000000",
            true,
            memeloop_workspace_control::auth::ApiKeyScope::initial_key_defaults(true),
            31_536_000,
            1,
        )
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "PostgreSQL CPU overcommit".to_owned(),
                owner_user_id: admin.user_id,
            },
            2,
        )
        .await
        .unwrap();
    let template = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(organization.id),
                yaml: WorkspaceTemplateDocument::new(
                    "PostgreSQL CPU overcommit",
                    WorkspaceTemplateSpec::standard(
                        OLD_IMAGE,
                        AccessMode::Internal,
                        Resources {
                            cpu_millis: 1_000,
                            memory_mib: 2_048,
                            gpu_count: 0,
                            disk_gib: 20,
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
    let workspace = database
        .create_workspace(
            CreateWorkspace {
                organization_id: organization.id,
                owner_id: admin.user_id,
                name: "PostgreSQL stopped CPU overcommit".to_owned(),
                template_id: template.id,
                resources: None,
                temporary_storage_gib: None,
                use_longhorn: false,
                organization_injection_refs: None,
                user_injection_refs: None,
            },
            true,
            admin.user_id,
            4,
        )
        .await
        .unwrap();
    let stopping = database
        .request_workspace_action(workspace.id, WorkspaceAction::Stop, admin.user_id, 5)
        .await
        .unwrap();
    let stopped = database
        .record_workspace_observation(
            stopping.id,
            stopping.generation,
            WorkspaceObservation::Stopped,
            admin.user_id,
            6,
        )
        .await
        .unwrap()
        .unwrap();

    let updated = database
        .update_stopped_workspace_cpu_overcommit(
            stopped.id,
            20,
            stopped.generation,
            admin.user_id,
            7,
        )
        .await
        .unwrap();
    assert_eq!(updated.template.cpu_overcommit_ratio, Some(20));
    let mut expected = stopped.template.clone();
    expected.cpu_overcommit_ratio = Some(20);
    assert_eq!(updated.template, expected);
    assert_eq!(updated.generation, stopped.generation + 1);
    assert!(matches!(
        database
            .update_stopped_workspace_cpu_overcommit(
                stopped.id,
                20,
                stopped.generation,
                admin.user_id,
                8
            )
            .await,
        Err(StorageError::WorkspaceCpuOvercommitUpdateConflict)
    ));
    let events = database
        .list_events(organization.id, Some(stopped.id), 20)
        .await
        .unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.payload["action"] == "cpu_overcommit_updated")
    );

    drop(database);
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&administration)
        .await
        .unwrap();
}
