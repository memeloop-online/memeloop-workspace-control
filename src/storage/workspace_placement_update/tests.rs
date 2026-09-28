use sqlx::Row;

use crate::{
    quota::Resources,
    storage::{CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate, PutNodePool},
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::{AccessMode, ResolvedPlacement, WorkspaceAction, WorkspaceObservation},
};

use super::*;

const OLD_IMAGE: &str = "registry.example/workspace@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const NEW_IMAGE: &str = "registry.example/workspace@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const NEW_POOL: &str = "haixia";

async fn stopped_workspace() -> (Database, Workspace, Uuid) {
    let database = Database::connect("sqlite::memory:", "placement-update".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
        .upsert_image_policy(OLD_IMAGE, true, 1)
        .await
        .unwrap();
    database
        .put_node_pool(
            NEW_POOL,
            &PutNodePool {
                display_name: "Haixia".to_owned(),
                placement: ResolvedPlacement {
                    required_hosts: vec!["haixia".to_owned()],
                    ..Default::default()
                },
                enabled: true,
            },
            1,
        )
        .await
        .unwrap();
    let admin = database
        .create_user_with_initial_key(
            "Admin",
            "placement-update-admin-000000000000000000",
            true,
            crate::auth::ApiKeyScope::initial_key_defaults(true),
            31_536_000,
            1,
        )
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Placement update".to_owned(),
                owner_user_id: admin.user_id,
            },
            2,
        )
        .await
        .unwrap();
    let yaml = WorkspaceTemplateDocument::new(
        "Placement update",
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
    .unwrap();
    let template = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(organization.id),
                yaml,
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
                name: "Stopped placement update".to_owned(),
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
    (database, stopped, admin.user_id)
}

async fn allow_new_pool(database: &Database, workspace: &Workspace) {
    let template = database
        .get_workspace_template(workspace.template_id.unwrap())
        .await
        .unwrap();
    let mut document = WorkspaceTemplateDocument::parse(&template.yaml).unwrap();
    document
        .spec
        .placement
        .allowed_node_pools
        .push(NEW_POOL.to_owned());
    document.spec.image = NEW_IMAGE.to_owned();
    document.spec.resources.disk_gib = 40;
    database
        .replace_workspace_template(template.id, &document.to_yaml().unwrap(), true, 7)
        .await
        .unwrap();
}

async fn snapshot(database: &Database, workspace: &Workspace) -> WorkspaceTemplateDocument {
    let Database::Sqlite {
        pool,
        installation_id,
    } = database
    else {
        unreachable!();
    };
    let yaml: String = sqlx::query_scalar(
        "SELECT template_snapshot_yaml FROM workspaces WHERE installation_id = ?1 AND id = ?2",
    )
    .bind(installation_id.as_str())
    .bind(workspace.id.to_string())
    .fetch_one(pool)
    .await
    .unwrap();
    WorkspaceTemplateDocument::parse(&yaml).unwrap()
}

#[tokio::test]
async fn newly_allowed_pool_updates_snapshot_policy_only_and_records_side_effects() {
    let (database, stopped, admin) = stopped_workspace().await;
    let original = snapshot(&database, &stopped).await;
    assert_eq!(original.spec.placement.allowed_node_pools, vec!["default"]);
    allow_new_pool(&database, &stopped).await;
    let updated = database
        .update_stopped_workspace_placement(stopped.id, NEW_POOL, stopped.generation, admin, 8)
        .await
        .unwrap();
    assert_eq!(updated.node_pool, NEW_POOL);
    assert_eq!(updated.generation, stopped.generation + 1);
    assert_eq!(updated.state, WorkspaceState::Stopped);
    assert!(
        updated
            .template
            .placement
            .allowed_node_pools
            .contains(&NEW_POOL.to_owned())
    );
    let persisted = snapshot(&database, &updated).await;
    assert_eq!(persisted.spec.placement, updated.template.placement);
    let mut expected = original;
    expected.spec.placement = persisted.spec.placement.clone();
    assert_eq!(persisted, expected);
    assert_eq!(updated.template.image, OLD_IMAGE);
    assert_eq!(updated.template.resources.disk_gib, 20);

    let Database::Sqlite {
        pool,
        installation_id,
    } = &database
    else {
        unreachable!();
    };
    let row = sqlx::query(
        "SELECT node_pool, generation FROM workspaces WHERE installation_id = ?1 AND id = ?2",
    )
    .bind(installation_id.as_str())
    .bind(updated.id.to_string())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(row.try_get::<String, _>("node_pool").unwrap(), NEW_POOL);
    assert_eq!(
        row.try_get::<i64, _>("generation").unwrap(),
        updated.generation as i64
    );
    let events = database
        .list_events(updated.organization_id, Some(updated.id), 20)
        .await
        .unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.kind == "workspace.placement_updated")
    );
    let job_payload: String = sqlx::query_scalar(
        "SELECT payload_json FROM jobs WHERE installation_id = ?1 AND workspace_id = ?2 AND kind = 'reconcile_workspace' ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(installation_id.as_str())
    .bind(updated.id.to_string())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&job_payload).unwrap()["reason"],
        "placement_updated"
    );
    let audit_action: String = sqlx::query_scalar(
        "SELECT action FROM audit_log WHERE installation_id = ?1 AND workspace_id = ?2 ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(installation_id.as_str())
    .bind(updated.id.to_string())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(audit_action, "workspace.placement_updated");
}

#[tokio::test]
async fn rejects_not_allowed_disabled_and_stale_without_snapshot_change() {
    let (database, stopped, admin) = stopped_workspace().await;
    assert!(matches!(
        database
            .update_stopped_workspace_placement(stopped.id, NEW_POOL, stopped.generation, admin, 8)
            .await,
        Err(StorageError::NodePoolNotAllowed)
    ));
    allow_new_pool(&database, &stopped).await;
    let template_id = stopped.template_id.unwrap();
    database
        .set_workspace_template_enabled(template_id, false, true, 8)
        .await
        .unwrap();
    assert!(matches!(
        database
            .update_stopped_workspace_placement(stopped.id, NEW_POOL, stopped.generation, admin, 9)
            .await,
        Err(StorageError::TemplateNotFound)
    ));
    database
        .set_workspace_template_enabled(template_id, true, true, 10)
        .await
        .unwrap();
    assert!(matches!(
        database
            .update_stopped_workspace_placement(
                stopped.id,
                NEW_POOL,
                stopped.generation + 1,
                admin,
                11
            )
            .await,
        Err(StorageError::WorkspacePlacementUpdateConflict)
    ));
    assert_eq!(database.get_workspace(stopped.id).await.unwrap(), stopped);
    assert_eq!(
        snapshot(&database, &stopped)
            .await
            .spec
            .placement
            .allowed_node_pools,
        vec!["default"]
    );
}

#[tokio::test]
async fn rejects_template_from_another_organization() {
    let (database, stopped, admin) = stopped_workspace().await;
    allow_new_pool(&database, &stopped).await;
    let other = database
        .create_organization(
            CreateOrganization {
                name: "Other organization".to_owned(),
                owner_user_id: admin,
            },
            8,
        )
        .await
        .unwrap();
    let Database::Sqlite {
        pool,
        installation_id,
    } = &database
    else {
        unreachable!();
    };
    sqlx::query("UPDATE workspace_templates SET organization_id = ?1 WHERE installation_id = ?2 AND id = ?3")
        .bind(other.id.to_string())
        .bind(installation_id.as_str())
        .bind(stopped.template_id.unwrap().to_string())
        .execute(pool)
        .await
        .unwrap();
    assert!(matches!(
        database
            .update_stopped_workspace_placement(stopped.id, NEW_POOL, stopped.generation, admin, 9)
            .await,
        Err(StorageError::TemplateNotFound)
    ));
    assert_eq!(database.get_workspace(stopped.id).await.unwrap(), stopped);
}

#[tokio::test]
async fn rejects_missing_template_reference_and_disabled_target_pool() {
    let (database, stopped, admin) = stopped_workspace().await;
    allow_new_pool(&database, &stopped).await;
    database
        .put_node_pool(
            NEW_POOL,
            &PutNodePool {
                display_name: "Haixia".to_owned(),
                placement: ResolvedPlacement::default(),
                enabled: false,
            },
            8,
        )
        .await
        .unwrap();
    assert!(matches!(
        database
            .update_stopped_workspace_placement(stopped.id, NEW_POOL, stopped.generation, admin, 9)
            .await,
        Err(StorageError::NodePoolUnavailable)
    ));
    let Database::Sqlite {
        pool,
        installation_id,
    } = &database
    else {
        unreachable!();
    };
    sqlx::query("UPDATE workspaces SET template_id = NULL WHERE installation_id = ?1 AND id = ?2")
        .bind(installation_id.as_str())
        .bind(stopped.id.to_string())
        .execute(pool)
        .await
        .unwrap();
    assert!(matches!(
        database
            .update_stopped_workspace_placement(stopped.id, NEW_POOL, stopped.generation, admin, 10)
            .await,
        Err(StorageError::TemplateNotFound)
    ));
    assert_eq!(
        snapshot(&database, &stopped)
            .await
            .spec
            .placement
            .allowed_node_pools,
        vec!["default"]
    );
}
