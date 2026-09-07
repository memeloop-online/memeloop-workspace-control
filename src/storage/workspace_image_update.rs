use sqlx::{PgConnection, Row, SqliteConnection};
use uuid::Uuid;

use crate::{
    templates::WorkspaceTemplateDocument,
    workspaces::{Workspace, WorkspaceState},
};

use super::{
    Database, StorageError,
    image_policy_store::IMAGE_CONTRACT_VERSION,
    workspace_store::{decode_postgres, decode_sqlite, select_workspace_sql},
};

impl Database {
    /// Atomically changes the image in a stopped workspace's immutable template snapshot.
    pub async fn update_stopped_workspace_image(
        &self,
        workspace_id: Uuid,
        image: &str,
        expected_generation: u64,
        actor_user_id: Uuid,
        now: i64,
    ) -> Result<Workspace, StorageError> {
        validate_pinned_image(image)?;
        let update = ImageUpdate {
            image,
            expected_generation,
            actor_user_id,
            now,
        };
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                let row = sqlx::query(&select_workspace_sql("?1", "?2"))
                    .bind(installation_id.as_str())
                    .bind(workspace_id.to_string())
                    .fetch_optional(&mut *transaction)
                    .await?;
                let row = row.ok_or(StorageError::WorkspaceNotFound)?;
                let snapshot_yaml: String = row.try_get("template_snapshot_yaml")?;
                let mut workspace = decode_sqlite(row, installation_id)?;
                update_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    &mut workspace,
                    &snapshot_yaml,
                    update,
                )
                .await?;
                transaction.commit().await?;
                Ok(workspace)
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                let row = sqlx::query(&format!("{} FOR UPDATE", select_workspace_sql("$1", "$2")))
                    .bind(installation_id.as_str())
                    .bind(workspace_id.to_string())
                    .fetch_optional(&mut *transaction)
                    .await?;
                let row = row.ok_or(StorageError::WorkspaceNotFound)?;
                let snapshot_yaml: String = row.try_get("template_snapshot_yaml")?;
                let mut workspace = decode_postgres(row, installation_id)?;
                update_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    &mut workspace,
                    &snapshot_yaml,
                    update,
                )
                .await?;
                transaction.commit().await?;
                Ok(workspace)
            }
        }
    }
}

#[derive(Clone, Copy)]
struct ImageUpdate<'a> {
    image: &'a str,
    expected_generation: u64,
    actor_user_id: Uuid,
    now: i64,
}

async fn update_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace: &mut Workspace,
    snapshot_yaml: &str,
    update: ImageUpdate<'_>,
) -> Result<(), StorageError> {
    ensure_stopped_at_generation(workspace, update.expected_generation)?;
    ensure_image_allowed_sqlite(connection, installation_id, update.image).await?;
    let snapshot_yaml = apply_image(workspace, snapshot_yaml, update.image, update.now)?;
    let affected = sqlx::query("UPDATE workspaces SET image = ?1, template_snapshot_yaml = ?2, generation = ?3, updated_at = ?4 WHERE installation_id = ?5 AND id = ?6 AND state = 'stopped' AND generation = ?7")
        .bind(update.image)
        .bind(snapshot_yaml)
        .bind(as_i64(workspace.generation)?)
        .bind(update.now)
        .bind(installation_id)
        .bind(workspace.id.to_string())
        .bind(as_i64(update.expected_generation)?)
        .execute(&mut *connection)
        .await?
        .rows_affected();
    if affected != 1 {
        return Err(StorageError::WorkspaceImageUpdateConflict);
    }
    record_side_effects_sqlite(
        connection,
        installation_id,
        workspace,
        update.actor_user_id,
        update.now,
    )
    .await
}

async fn update_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &mut Workspace,
    snapshot_yaml: &str,
    update: ImageUpdate<'_>,
) -> Result<(), StorageError> {
    ensure_stopped_at_generation(workspace, update.expected_generation)?;
    ensure_image_allowed_postgres(connection, installation_id, update.image).await?;
    let snapshot_yaml = apply_image(workspace, snapshot_yaml, update.image, update.now)?;
    let affected = sqlx::query("UPDATE workspaces SET image = $1, template_snapshot_yaml = $2, generation = $3, updated_at = $4 WHERE installation_id = $5 AND id = $6 AND state = 'stopped' AND generation = $7")
        .bind(update.image)
        .bind(snapshot_yaml)
        .bind(as_i64(workspace.generation)?)
        .bind(update.now)
        .bind(installation_id)
        .bind(workspace.id.to_string())
        .bind(as_i64(update.expected_generation)?)
        .execute(&mut *connection)
        .await?
        .rows_affected();
    if affected != 1 {
        return Err(StorageError::WorkspaceImageUpdateConflict);
    }
    record_side_effects_postgres(
        connection,
        installation_id,
        workspace,
        update.actor_user_id,
        update.now,
    )
    .await
}

fn ensure_stopped_at_generation(
    workspace: &Workspace,
    expected_generation: u64,
) -> Result<(), StorageError> {
    (workspace.state == WorkspaceState::Stopped && workspace.generation == expected_generation)
        .then_some(())
        .ok_or(StorageError::WorkspaceImageUpdateConflict)
}

async fn ensure_image_allowed_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    image: &str,
) -> Result<(), StorageError> {
    let allowed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM image_policies WHERE installation_id = ?1 AND image = ?2 AND enabled = 1 AND contract_version = ?3")
        .bind(installation_id)
        .bind(image)
        .bind(i64::from(IMAGE_CONTRACT_VERSION))
        .fetch_one(&mut *connection)
        .await?;
    (allowed == 1)
        .then_some(())
        .ok_or(StorageError::ImageNotAllowed)
}

async fn ensure_image_allowed_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    image: &str,
) -> Result<(), StorageError> {
    let allowed = sqlx::query_scalar::<_, String>("SELECT image FROM image_policies WHERE installation_id = $1 AND image = $2 AND enabled = 1 AND contract_version = $3 FOR SHARE")
        .bind(installation_id)
        .bind(image)
        .bind(i64::from(IMAGE_CONTRACT_VERSION))
        .fetch_optional(&mut *connection)
        .await?;
    allowed.ok_or(StorageError::ImageNotAllowed).map(|_| ())
}

fn apply_image(
    workspace: &mut Workspace,
    snapshot_yaml: &str,
    image: &str,
    now: i64,
) -> Result<String, StorageError> {
    let mut snapshot = WorkspaceTemplateDocument::parse(snapshot_yaml)
        .map_err(|_| StorageError::InvalidWorkspace)?;
    snapshot.spec.image = image.to_owned();
    workspace.template.image = image.to_owned();
    workspace.generation = workspace
        .generation
        .checked_add(1)
        .ok_or(StorageError::InvalidWorkspace)?;
    workspace.updated_at = now;
    snapshot
        .to_yaml()
        .map_err(|_| StorageError::InvalidWorkspace)
}

async fn record_side_effects_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace: &Workspace,
    actor_user_id: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES (?1, ?2, 'reconcile_workspace', ?3, ?4, 'pending', ?5, NULL, NULL, 0, ?5, ?5)")
        .bind(Uuid::now_v7().to_string())
        .bind(installation_id)
        .bind(workspace.id.to_string())
        .bind(serde_json::json!({"generation": workspace.generation, "reason": "image_updated"}).to_string())
        .bind(now)
        .execute(&mut *connection)
        .await?;
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, 'workspace.image_updated', ?6, ?7)")
        .bind(Uuid::now_v7().to_string())
        .bind(installation_id)
        .bind(actor_user_id.to_string())
        .bind(workspace.organization_id.to_string())
        .bind(workspace.id.to_string())
        .bind(serde_json::json!({"image": workspace.template.image, "generation": workspace.generation}).to_string())
        .bind(now)
        .execute(&mut *connection)
        .await?;
    super::workspace_events::insert_image_updated_sqlite(
        connection,
        installation_id,
        workspace,
        now,
    )
    .await
}

async fn record_side_effects_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &Workspace,
    actor_user_id: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES ($1, $2, 'reconcile_workspace', $3, $4, 'pending', $5, NULL, NULL, 0, $5, $5)")
        .bind(Uuid::now_v7().to_string())
        .bind(installation_id)
        .bind(workspace.id.to_string())
        .bind(serde_json::json!({"generation": workspace.generation, "reason": "image_updated"}).to_string())
        .bind(now)
        .execute(&mut *connection)
        .await?;
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES ($1, $2, $3, $4, $5, 'workspace.image_updated', $6, $7)")
        .bind(Uuid::now_v7().to_string())
        .bind(installation_id)
        .bind(actor_user_id.to_string())
        .bind(workspace.organization_id.to_string())
        .bind(workspace.id.to_string())
        .bind(serde_json::json!({"image": workspace.template.image, "generation": workspace.generation}).to_string())
        .bind(now)
        .execute(&mut *connection)
        .await?;
    super::workspace_events::insert_image_updated_postgres(
        connection,
        installation_id,
        workspace,
        now,
    )
    .await
}

fn validate_pinned_image(image: &str) -> Result<(), StorageError> {
    if image.is_empty()
        || image != image.trim()
        || image.len() > 512
        || image.chars().any(char::is_whitespace)
    {
        return Err(StorageError::InvalidWorkspaceImageUpdate);
    }
    let Some((repository, digest)) = image.split_once("@sha256:") else {
        return Err(StorageError::InvalidWorkspaceImageUpdate);
    };
    if repository.is_empty()
        || repository.contains('@')
        || digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (byte as char).is_ascii_lowercase())
    {
        return Err(StorageError::InvalidWorkspaceImageUpdate);
    }
    Ok(())
}

fn as_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::InvalidWorkspace)
}

#[cfg(test)]
mod tests {
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
            .create_user("Admin", "image-update-admin-000000000000000000000", true, 1)
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
            .record_workspace_observation(
                stopping.id,
                WorkspaceObservation::Stopped,
                admin.user_id,
                6,
            )
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
    async fn rejects_unpinned_or_stale_updates_without_writing() {
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
        assert!(matches!(
            database
                .update_stopped_workspace_image(
                    stopped.id,
                    NEW_IMAGE,
                    stopped.generation + 1,
                    admin,
                    7
                )
                .await,
            Err(StorageError::WorkspaceImageUpdateConflict)
        ));
        let unchanged = database.get_workspace(stopped.id).await.unwrap();
        assert_eq!(unchanged.template.image, OLD_IMAGE);
        assert_eq!(unchanged.generation, stopped.generation);
    }
}
