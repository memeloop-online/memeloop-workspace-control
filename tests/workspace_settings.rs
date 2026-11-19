use memeloop_workspace_control::{
    auth::ApiKeyScope,
    quota::{QuotaResources, Resources},
    storage::{
        CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate, Database, StorageError,
        WorkspaceSettingsUpdate,
    },
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::{AccessMode, Workspace, WorkspaceAction, WorkspaceObservation},
};
use uuid::Uuid;

fn update(workspace: &Workspace, cpu_millis: u64) -> WorkspaceSettingsUpdate {
    WorkspaceSettingsUpdate {
        name: "更新后的工作区".to_owned(),
        cpu_millis,
        memory_mib: workspace.template.resources.memory_mib,
        gpu_count: workspace.template.resources.gpu_count,
        expected_generation: workspace.generation,
        actor_user_id: workspace.owner_id,
        now: 20,
    }
}

async fn snapshot(database: &Database, id: Uuid) -> String {
    match database {
        Database::Sqlite { pool, .. } => {
            sqlx::query_scalar("SELECT template_snapshot_yaml FROM workspaces WHERE id = ?1")
                .bind(id.to_string())
                .fetch_one(pool)
                .await
                .unwrap()
        }
        Database::Postgres { pool, .. } => {
            sqlx::query_scalar("SELECT template_snapshot_yaml FROM workspaces WHERE id = $1")
                .bind(id.to_string())
                .fetch_one(pool)
                .await
                .unwrap()
        }
    }
}

async fn exercise(database: Database) {
    database.migrate().await.unwrap();
    let image = "registry.example/settings:1";
    database.upsert_image_policy(image, true, 1).await.unwrap();
    let user = database
        .create_user_with_initial_key(
            "Admin",
            "workspace-settings-admin-000000000000000000000000",
            true,
            ApiKeyScope::initial_key_defaults(true),
            31_536_000,
            1,
        )
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Settings".to_owned(),
                owner_user_id: user.user_id,
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
                    "Settings",
                    WorkspaceTemplateSpec::standard(
                        image,
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
    let created = database
        .create_workspace(
            CreateWorkspace {
                organization_id: organization.id,
                owner_id: user.user_id,
                name: "Original".to_owned(),
                template_id: template.id,
                resources: None,
                temporary_storage_gib: None,
                use_longhorn: false,
                organization_injection_refs: None,
                user_injection_refs: None,
            },
            true,
            user.user_id,
            4,
        )
        .await
        .unwrap();
    let ready = database
        .record_workspace_observation(
            created.id,
            created.generation,
            WorkspaceObservation::Ready,
            user.user_id,
            5,
        )
        .await
        .unwrap()
        .unwrap();
    let original_yaml = snapshot(&database, ready.id).await;
    let jobs = database.job_counts().await.unwrap().pending;
    let renamed = database
        .update_workspace_settings(ready.id, update(&ready, 1_000))
        .await
        .unwrap();
    assert_eq!(renamed.name, "更新后的工作区");
    assert_eq!(renamed.generation, ready.generation);
    assert_eq!(renamed.template, ready.template);
    assert_eq!(database.job_counts().await.unwrap().pending, jobs);
    assert_eq!(snapshot(&database, ready.id).await, original_yaml);
    assert!(matches!(
        database
            .update_workspace_settings(ready.id, update(&renamed, 2_000))
            .await,
        Err(StorageError::WorkspaceSettingsUpdateConflict)
    ));
    assert_eq!(database.get_workspace(ready.id).await.unwrap(), renamed);

    let stopping = database
        .request_workspace_action(ready.id, WorkspaceAction::Stop, user.user_id, 21)
        .await
        .unwrap();
    let stopped = database
        .record_workspace_observation(
            ready.id,
            stopping.generation,
            WorkspaceObservation::Stopped,
            user.user_id,
            22,
        )
        .await
        .unwrap()
        .unwrap();
    // Exactly the replacement allocation fits. Counting the previous 1-core allocation twice would reject it.
    let quota = QuotaResources {
        cpu_millis: 2_000,
        memory_mib: 2_048,
        gpu_count: 0,
        disk_gib: 20,
        ..Default::default()
    };
    database
        .set_organization_quota(organization.id, quota, 23)
        .await
        .unwrap();
    database
        .set_user_quota(user.user_id, quota, 23)
        .await
        .unwrap();
    let jobs = database.job_counts().await.unwrap().pending;
    let updated = database
        .update_workspace_settings(stopped.id, update(&stopped, 2_000))
        .await
        .unwrap();
    assert_eq!(updated.generation, stopped.generation + 1);
    assert_eq!(database.job_counts().await.unwrap().pending, jobs + 1);
    let mut expected = stopped.template.clone();
    expected.resources.cpu_millis = 2_000;
    assert_eq!(updated.template, expected);
    assert_eq!(updated.id, stopped.id);
    assert_eq!(updated.runtime, stopped.runtime);
    assert_eq!(updated.node_pool, stopped.node_pool);
    assert_eq!(updated.home_volume_binding, stopped.home_volume_binding);
    assert_eq!(
        updated.scratch_volume_binding,
        stopped.scratch_volume_binding
    );
    let updated_yaml = snapshot(&database, updated.id).await;
    assert_eq!(
        WorkspaceTemplateDocument::parse(&updated_yaml)
            .unwrap()
            .spec,
        expected
    );
    let events = database
        .list_events(organization.id, Some(updated.id), 20)
        .await
        .unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.payload["action"] == "settings_updated")
    );
    assert!(matches!(
        database
            .update_workspace_settings(stopped.id, update(&stopped, 2_000))
            .await,
        Err(StorageError::WorkspaceSettingsUpdateConflict)
    ));

    // Both quota paths must roll back the attempted name and snapshot alongside the resource change.
    for organization_limited in [true, false] {
        let generous = QuotaResources {
            cpu_millis: 4_000,
            ..quota
        };
        database
            .set_organization_quota(
                organization.id,
                if organization_limited {
                    quota
                } else {
                    generous
                },
                24,
            )
            .await
            .unwrap();
        database
            .set_user_quota(
                user.user_id,
                if organization_limited {
                    generous
                } else {
                    quota
                },
                24,
            )
            .await
            .unwrap();
        let mut command = update(&updated, 3_000);
        command.name = "Must roll back".to_owned();
        assert!(matches!(
            database
                .update_workspace_settings(updated.id, command)
                .await,
            Err(StorageError::Quota(_))
        ));
        assert_eq!(database.get_workspace(updated.id).await.unwrap(), updated);
        assert_eq!(snapshot(&database, updated.id).await, updated_yaml);
        assert_eq!(database.job_counts().await.unwrap().pending, jobs + 1);
    }
    let starting = database
        .request_workspace_action(updated.id, WorkspaceAction::Start, user.user_id, 25)
        .await
        .unwrap();
    assert_eq!(starting.template, expected);
}

#[tokio::test]
async fn sqlite_workspace_settings_are_atomic_and_preserve_storage() {
    exercise(
        Database::connect("sqlite::memory:", "settings-sqlite".parse().unwrap())
            .await
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn postgres_workspace_settings_are_atomic_and_preserve_storage() {
    let Ok(url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        return;
    };
    let schema = format!("mwc_settings_{}", Uuid::now_v7().simple());
    let administration = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&administration)
        .await
        .unwrap();
    let mut scoped = url::Url::parse(&url).unwrap();
    scoped
        .query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema}"));
    let database = Database::connect(scoped.as_str(), "settings-pg".parse().unwrap())
        .await
        .unwrap();
    // Collect a test panic before cleanup so even a regression does not leave its private schema behind.
    let result = tokio::spawn(exercise(database)).await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&administration)
        .await
        .unwrap();
    result.unwrap();
}
