use sqlx::Row;

use crate::{
    quota::Resources,
    storage::{CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate},
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::{AccessMode, WorkspaceAction, WorkspaceObservation},
};

use super::*;

const OLD_IMAGE: &str = "registry.example/workspace@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const NEW_IMAGE: &str = "registry.example/workspace@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

async fn stopped_workspace() -> (Database, Workspace, Uuid) {
    let database = Database::connect("sqlite::memory:", "image-update-test".parse().unwrap())
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
        .create_user_with_initial_key(
            "Admin",
            "image-update-admin-000000000000000000000",
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
                name: "Image update".to_owned(),
                owner_user_id: admin.user_id,
            },
            2,
        )
        .await
        .unwrap();
    let yaml = WorkspaceTemplateDocument::new(
        "Image update",
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
                name: "Stopped image update".to_owned(),
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

#[tokio::test]
async fn updates_both_image_copies_and_records_image_specific_side_effects() {
    let (database, stopped, admin) = stopped_workspace().await;
    let updated = database
        .update_stopped_workspace_image(stopped.id, NEW_IMAGE, stopped.generation, admin, 7)
        .await
        .unwrap();
    assert_eq!(updated.template.image, NEW_IMAGE);
    assert_eq!(updated.state, WorkspaceState::Stopped);
    assert_eq!(updated.generation, stopped.generation + 1);

    let Database::Sqlite {
        pool,
        installation_id,
    } = &database
    else {
        unreachable!();
    };
    let row = sqlx::query("SELECT image, template_snapshot_yaml FROM workspaces WHERE installation_id = ?1 AND id = ?2")
        .bind(installation_id.as_str())
        .bind(updated.id.to_string())
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(row.try_get::<String, _>("image").unwrap(), NEW_IMAGE);
    let snapshot: String = row.try_get("template_snapshot_yaml").unwrap();
    assert_eq!(
        WorkspaceTemplateDocument::parse(&snapshot)
            .unwrap()
            .spec
            .image,
        NEW_IMAGE
    );
    let events = database
        .list_events(updated.organization_id, Some(updated.id), 20)
        .await
        .unwrap();
    let event = events
        .iter()
        .find(|event| event.kind == "workspace.image_updated")
        .unwrap();
    assert_eq!(event.payload["image"], NEW_IMAGE);
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
        "image_updated"
    );
    let audit_action: String = sqlx::query_scalar(
        "SELECT action FROM audit_log WHERE installation_id = ?1 AND workspace_id = ?2 ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(installation_id.as_str())
    .bind(updated.id.to_string())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(audit_action, "workspace.image_updated");
}

#[tokio::test]
async fn rejects_unpinned_nonhex_or_stale_updates_without_writing() {
    let (database, stopped, admin) = stopped_workspace().await;
    assert!(matches!(
        database
            .update_stopped_workspace_image(
                stopped.id,
                "registry.example/workspace:next",
                stopped.generation,
                admin,
                7
            )
            .await,
        Err(StorageError::InvalidWorkspaceImageUpdate)
    ));
    let nonhex_digest = format!("registry.example/workspace@sha256:{}g", "a".repeat(63));
    assert!(matches!(
        database
            .update_stopped_workspace_image(
                stopped.id,
                &nonhex_digest,
                stopped.generation,
                admin,
                7
            )
            .await,
        Err(StorageError::InvalidWorkspaceImageUpdate)
    ));
    assert!(matches!(
        database
            .update_stopped_workspace_image(stopped.id, NEW_IMAGE, stopped.generation + 1, admin, 7)
            .await,
        Err(StorageError::WorkspaceImageUpdateConflict)
    ));
    let unchanged = database.get_workspace(stopped.id).await.unwrap();
    assert_eq!(unchanged.template.image, OLD_IMAGE);
    assert_eq!(unchanged.generation, stopped.generation);
}
