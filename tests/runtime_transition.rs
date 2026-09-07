use memeloop_workspace_control::{
    quota::Resources,
    storage::{
        CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate, Database, StorageError,
    },
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspace_runtime::WorkspaceRuntimeNamingScheme,
    workspaces::AccessMode,
};

const TOKEN: &str = "runtime-transition-test-token-000000000000000000";

async fn seeded_workspace() -> (Database, memeloop_workspace_control::workspaces::Workspace) {
    let database = Database::connect("sqlite::memory:", "runtime-transition".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
        .upsert_image_policy("registry.example/workspace:1", true, 1)
        .await
        .unwrap();
    let user = database
        .create_user("Runtime transition", TOKEN, true, 2)
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Runtime transition".to_owned(),
                owner_user_id: user.user_id,
            },
            3,
        )
        .await
        .unwrap();
    let template = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(organization.id),
                yaml: WorkspaceTemplateDocument::new(
                    "Runtime transition",
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
                )
                .to_yaml()
                .unwrap(),
            },
            true,
            4,
        )
        .await
        .unwrap();
    let workspace = database
        .create_workspace(
            CreateWorkspace {
                organization_id: organization.id,
                owner_id: user.user_id,
                name: "runtime-transition".to_owned(),
                template_id: template.id,
                resources: None,
                organization_injection_refs: None,
                user_injection_refs: None,
            },
            true,
            user.user_id,
            5,
        )
        .await
        .unwrap();
    (database, workspace)
}

#[tokio::test]
async fn stopped_workspace_is_canonicalized_once_and_enqueued() {
    let (database, workspace) = seeded_workspace().await;
    let Database::Sqlite { pool, .. } = &database else {
        unreachable!();
    };
    sqlx::query("DELETE FROM jobs").execute(pool).await.unwrap();
    sqlx::query(
        "UPDATE workspaces SET state = 'stopped', runtime_naming_scheme = 'legacy_v1', \
         runtime_namespace_scope = 'dedicated', runtime_resource_prefix = 'workspace', \
         runtime_route_key = ?1 WHERE id = ?2",
    )
    .bind(&workspace.short_id)
    .bind(workspace.id.to_string())
    .execute(pool)
    .await
    .unwrap();

    let migrated = database
        .canonicalize_workspace_runtime(workspace.id, 10)
        .await
        .unwrap();
    assert_eq!(
        migrated.runtime.naming_scheme,
        WorkspaceRuntimeNamingScheme::PrefixedV2
    );
    assert_eq!(
        migrated.runtime.resource_prefix,
        format!("w-{}", workspace.short_id)
    );
    assert_eq!(migrated.generation, workspace.generation + 1);

    let job_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM jobs WHERE workspace_id = ?1 AND status = 'pending'",
    )
    .bind(workspace.id.to_string())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(job_count, 1);

    let repeated = database
        .canonicalize_workspace_runtime(workspace.id, 11)
        .await
        .unwrap();
    assert_eq!(repeated.generation, migrated.generation);
    let job_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE workspace_id = ?1")
        .bind(workspace.id.to_string())
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(job_count, 1);
}

#[tokio::test]
async fn running_workspace_or_active_job_blocks_transition() {
    let (database, workspace) = seeded_workspace().await;
    assert!(matches!(
        database
            .canonicalize_workspace_runtime(workspace.id, 10)
            .await,
        Err(StorageError::WorkspaceRuntimeMigrationUnsafe)
    ));

    let Database::Sqlite { pool, .. } = &database else {
        unreachable!();
    };
    sqlx::query(
        "UPDATE workspaces SET state = 'stopped', runtime_naming_scheme = 'legacy_v1', \
         runtime_namespace_scope = 'dedicated', runtime_resource_prefix = 'workspace', \
         runtime_route_key = ?1 WHERE id = ?2",
    )
    .bind(&workspace.short_id)
    .bind(workspace.id.to_string())
    .execute(pool)
    .await
    .unwrap();
    assert!(matches!(
        database
            .canonicalize_workspace_runtime(workspace.id, 11)
            .await,
        Err(StorageError::WorkspaceRuntimeMigrationUnsafe)
    ));
}
