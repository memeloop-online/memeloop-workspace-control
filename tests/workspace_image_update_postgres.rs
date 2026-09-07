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
const NEW_IMAGE: &str = "registry.example/workspace@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[tokio::test]
async fn postgres_updates_a_locked_stopped_workspace_image() {
    let Ok(database_url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        eprintln!("skipping PostgreSQL image update test: MWC_TEST_POSTGRES_URL is not set");
        return;
    };
    let schema = format!("mwc_image_update_{}", Uuid::now_v7().simple());
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
    let installation_id = format!("image-pg-{suffix}").parse().unwrap();
    let database = Database::connect(scoped_url.as_str(), installation_id)
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
        .upsert_image_policy(OLD_IMAGE, true, 1)
        .await
        .unwrap();
    database
        .upsert_image_policy(NEW_IMAGE, true, 2)
        .await
        .unwrap();
    let admin = database
        .create_user(
            "Admin",
            "image-update-pg-admin-000000000000000000000000",
            true,
            1,
        )
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "PostgreSQL image update".to_owned(),
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
                    "PostgreSQL image update",
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
                name: "PostgreSQL stopped image update".to_owned(),
                template_id: template.id,
                resources: None,
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
        .record_workspace_observation(stopping.id, WorkspaceObservation::Stopped, admin.user_id, 6)
        .await
        .unwrap();

    let updated = database
        .update_stopped_workspace_image(stopped.id, NEW_IMAGE, stopped.generation, admin.user_id, 7)
        .await
        .unwrap();
    assert_eq!(updated.template.image, NEW_IMAGE);
    assert_eq!(updated.generation, stopped.generation + 1);
    assert!(matches!(
        database
            .update_stopped_workspace_image(
                stopped.id,
                NEW_IMAGE,
                stopped.generation,
                admin.user_id,
                8
            )
            .await,
        Err(StorageError::WorkspaceImageUpdateConflict)
    ));
    let events = database
        .list_events(organization.id, Some(stopped.id), 20)
        .await
        .unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.kind == "workspace.image_updated")
    );

    drop(database);
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&administration)
        .await
        .unwrap();
}
