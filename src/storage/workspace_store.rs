use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, Row, SqliteConnection, postgres::PgRow, sqlite::SqliteRow};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    crypto::EnvelopeCipher,
    injections::InjectionItem,
    templates::WorkspaceTemplateDocument,
    workspaces::{Workspace, WorkspaceState},
};

use super::{Database, StorageError, WorkspaceInjectionRefs};

const WORKSPACE_COLUMNS: &str = "id, short_id, organization_id, owner_id, name, template_id, \
    template_snapshot_yaml, state, generation, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateWorkspace {
    pub organization_id: Uuid,
    pub owner_id: Uuid,
    pub name: String,
    pub template_id: Uuid,
    #[serde(default)]
    pub organization_injection_refs: Option<Vec<String>>,
    #[serde(default)]
    pub user_injection_refs: Option<Vec<String>>,
}

impl Database {
    pub async fn create_workspace(
        &self,
        command: CreateWorkspace,
        actor_user_id: Uuid,
        now: i64,
    ) -> Result<Workspace, StorageError> {
        self.create_workspace_inner(command, None, actor_user_id, now)
            .await
    }

    pub async fn create_workspace_with_inline_injections(
        &self,
        command: CreateWorkspace,
        cipher: &EnvelopeCipher,
        inline: &[InjectionItem],
        actor_user_id: Uuid,
        now: i64,
    ) -> Result<Workspace, StorageError> {
        self.create_workspace_inner(command, Some((cipher, inline)), actor_user_id, now)
            .await
    }

    async fn create_workspace_inner(
        &self,
        command: CreateWorkspace,
        inline: Option<(&EnvelopeCipher, &[InjectionItem])>,
        actor_user_id: Uuid,
        now: i64,
    ) -> Result<Workspace, StorageError> {
        if command.name.trim().is_empty() || command.name.len() > 120 {
            return Err(StorageError::InvalidWorkspace);
        }
        let injection_refs = WorkspaceInjectionRefs {
            organization: command.organization_injection_refs.clone(),
            user: command.user_injection_refs.clone(),
        };
        injection_refs.validate()?;
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                let workspace = create_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    &command,
                    actor_user_id,
                    &injection_refs,
                    inline,
                    now,
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
                let workspace = create_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    &command,
                    actor_user_id,
                    &injection_refs,
                    inline,
                    now,
                )
                .await?;
                transaction.commit().await?;
                Ok(workspace)
            }
        }
    }

    pub async fn get_workspace(&self, workspace_id: Uuid) -> Result<Workspace, StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(&select_workspace_sql("?1", "?2"))
                .bind(installation_id.as_str())
                .bind(workspace_id.to_string())
                .fetch_optional(pool)
                .await?
                .map(decode_sqlite)
                .transpose()?
                .ok_or(StorageError::WorkspaceNotFound),
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(&select_workspace_sql("$1", "$2"))
                .bind(installation_id.as_str())
                .bind(workspace_id.to_string())
                .fetch_optional(pool)
                .await?
                .map(decode_postgres)
                .transpose()?
                .ok_or(StorageError::WorkspaceNotFound),
        }
    }

    pub async fn list_workspaces(
        &self,
        organization_id: Uuid,
    ) -> Result<Vec<Workspace>, StorageError> {
        let sql = format!(
            "SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE installation_id = {{install}} AND organization_id = {{organization}} AND state <> 'deleted' ORDER BY created_at, id"
        );
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(
                &sql.replace("{install}", "?1")
                    .replace("{organization}", "?2"),
            )
            .bind(installation_id.as_str())
            .bind(organization_id.to_string())
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(decode_sqlite)
            .collect(),
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(
                &sql.replace("{install}", "$1")
                    .replace("{organization}", "$2"),
            )
            .bind(installation_id.as_str())
            .bind(organization_id.to_string())
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(decode_postgres)
            .collect(),
        }
    }
}

fn build_workspace(
    command: &CreateWorkspace,
    snapshot: super::catalog_store::ResolvedTemplateSnapshot,
    now: i64,
) -> Workspace {
    let id = Uuid::now_v7();
    Workspace {
        id,
        short_id: id.simple().to_string()[..8].to_owned(),
        organization_id: command.organization_id,
        owner_id: command.owner_id,
        name: command.name.trim().to_owned(),
        template_id: Some(command.template_id),
        template: snapshot.spec,
        state: WorkspaceState::Provisioning,
        generation: 1,
        created_at: now,
        updated_at: now,
    }
}

async fn create_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    command: &CreateWorkspace,
    actor_user_id: Uuid,
    injection_refs: &WorkspaceInjectionRefs,
    inline: Option<(&EnvelopeCipher, &[InjectionItem])>,
    now: i64,
) -> Result<Workspace, StorageError> {
    let snapshot = super::catalog_store::resolve_template_sqlite(
        connection,
        installation_id,
        command.template_id,
        command.organization_id,
    )
    .await?;
    let yaml = snapshot.yaml.clone();
    let workspace = build_workspace(command, snapshot, now);
    super::workspace_admission::admit_sqlite(connection, installation_id, &workspace).await?;
    insert_sqlite(connection, installation_id, &workspace, &yaml, now).await?;
    insert_injections_sqlite(
        connection,
        installation_id,
        &workspace,
        actor_user_id,
        injection_refs,
        inline,
        now,
    )
    .await?;
    enqueue_and_audit_sqlite(connection, installation_id, &workspace, actor_user_id, now).await?;
    Ok(workspace)
}

async fn create_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    command: &CreateWorkspace,
    actor_user_id: Uuid,
    injection_refs: &WorkspaceInjectionRefs,
    inline: Option<(&EnvelopeCipher, &[InjectionItem])>,
    now: i64,
) -> Result<Workspace, StorageError> {
    let snapshot = super::catalog_store::resolve_template_postgres(
        connection,
        installation_id,
        command.template_id,
        command.organization_id,
    )
    .await?;
    let yaml = snapshot.yaml.clone();
    let workspace = build_workspace(command, snapshot, now);
    super::workspace_admission::admit_postgres(connection, installation_id, &workspace).await?;
    insert_postgres(connection, installation_id, &workspace, &yaml, now).await?;
    insert_injections_postgres(
        connection,
        installation_id,
        &workspace,
        actor_user_id,
        injection_refs,
        inline,
        now,
    )
    .await?;
    enqueue_and_audit_postgres(connection, installation_id, &workspace, actor_user_id, now).await?;
    Ok(workspace)
}

async fn insert_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace: &Workspace,
    yaml: &str,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO workspaces (id, installation_id, short_id, organization_id, owner_id, name, template_id, image, access_mode, state, cpu_millis, memory_mib, gpu_count, disk_gib, generation, created_at, updated_at, deleted_at, template_snapshot_yaml) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,1,?15,?15,NULL,?16)")
        .bind(workspace.id.to_string()).bind(installation_id).bind(&workspace.short_id).bind(workspace.organization_id.to_string()).bind(workspace.owner_id.to_string()).bind(&workspace.name).bind(workspace.template_id.map(|id| id.to_string())).bind(&workspace.template.image).bind(workspace.template.access_mode.as_str()).bind(workspace.state.as_str()).bind(as_i64(workspace.template.resources.cpu_millis)?).bind(as_i64(workspace.template.resources.memory_mib)?).bind(i64::from(workspace.template.resources.gpu_count)).bind(as_i64(workspace.template.resources.disk_gib)?).bind(now).bind(yaml).execute(&mut *connection).await?;
    Ok(())
}

async fn insert_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &Workspace,
    yaml: &str,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO workspaces (id, installation_id, short_id, organization_id, owner_id, name, template_id, image, access_mode, state, cpu_millis, memory_mib, gpu_count, disk_gib, generation, created_at, updated_at, deleted_at, template_snapshot_yaml) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,1,$15,$15,NULL,$16)")
        .bind(workspace.id.to_string()).bind(installation_id).bind(&workspace.short_id).bind(workspace.organization_id.to_string()).bind(workspace.owner_id.to_string()).bind(&workspace.name).bind(workspace.template_id.map(|id| id.to_string())).bind(&workspace.template.image).bind(workspace.template.access_mode.as_str()).bind(workspace.state.as_str()).bind(as_i64(workspace.template.resources.cpu_millis)?).bind(as_i64(workspace.template.resources.memory_mib)?).bind(i64::from(workspace.template.resources.gpu_count)).bind(as_i64(workspace.template.resources.disk_gib)?).bind(now).bind(yaml).execute(&mut *connection).await?;
    Ok(())
}

async fn insert_injections_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace: &Workspace,
    actor: Uuid,
    refs: &WorkspaceInjectionRefs,
    inline: Option<(&EnvelopeCipher, &[InjectionItem])>,
    now: i64,
) -> Result<(), StorageError> {
    super::workspace_injection_refs::insert_sqlite(
        connection,
        installation_id,
        workspace.id,
        refs,
        now,
    )
    .await?;
    if let Some((cipher, items)) = inline {
        for item in items {
            super::injection_store::insert_initial_workspace_injection_sqlite(
                connection,
                cipher,
                installation_id,
                workspace.id,
                item,
                actor,
                now,
            )
            .await?;
        }
    }
    Ok(())
}

async fn insert_injections_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &Workspace,
    actor: Uuid,
    refs: &WorkspaceInjectionRefs,
    inline: Option<(&EnvelopeCipher, &[InjectionItem])>,
    now: i64,
) -> Result<(), StorageError> {
    super::workspace_injection_refs::insert_postgres(
        connection,
        installation_id,
        workspace.id,
        refs,
        now,
    )
    .await?;
    if let Some((cipher, items)) = inline {
        for item in items {
            super::injection_store::insert_initial_workspace_injection_postgres(
                connection,
                cipher,
                installation_id,
                workspace.id,
                item,
                actor,
                now,
            )
            .await?;
        }
    }
    Ok(())
}

async fn enqueue_and_audit_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace: &Workspace,
    actor: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES (?1,?2,'reconcile_workspace',?3,?4,'pending',?5,NULL,NULL,0,?5,?5)").bind(Uuid::now_v7().to_string()).bind(installation_id).bind(workspace.id.to_string()).bind(serde_json::json!({"generation": workspace.generation}).to_string()).bind(now).execute(&mut *connection).await?;
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES (?1,?2,?3,?4,?5,'workspace.create',?6,?7)").bind(Uuid::now_v7().to_string()).bind(installation_id).bind(actor.to_string()).bind(workspace.organization_id.to_string()).bind(workspace.id.to_string()).bind(serde_json::json!({"name": workspace.name, "image": workspace.template.image}).to_string()).bind(now).execute(&mut *connection).await?;
    super::workspace_events::insert_sqlite(connection, installation_id, workspace, None, now).await
}

async fn enqueue_and_audit_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &Workspace,
    actor: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES ($1,$2,'reconcile_workspace',$3,$4,'pending',$5,NULL,NULL,0,$5,$5)").bind(Uuid::now_v7().to_string()).bind(installation_id).bind(workspace.id.to_string()).bind(serde_json::json!({"generation": workspace.generation}).to_string()).bind(now).execute(&mut *connection).await?;
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES ($1,$2,$3,$4,$5,'workspace.create',$6,$7)").bind(Uuid::now_v7().to_string()).bind(installation_id).bind(actor.to_string()).bind(workspace.organization_id.to_string()).bind(workspace.id.to_string()).bind(serde_json::json!({"name": workspace.name, "image": workspace.template.image}).to_string()).bind(now).execute(&mut *connection).await?;
    super::workspace_events::insert_postgres(connection, installation_id, workspace, None, now)
        .await
}

pub(super) fn select_workspace_sql(installation: &str, id: &str) -> String {
    format!(
        "SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE installation_id = {installation} AND id = {id}"
    )
}

pub(super) fn select_workspace_by_short_id_sql(installation: &str, short_id: &str) -> String {
    format!(
        "SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE installation_id = {installation} AND short_id = {short_id}"
    )
}

pub(super) fn decode_sqlite(row: SqliteRow) -> Result<Workspace, StorageError> {
    decode_workspace(&row)
}
pub(super) fn decode_postgres(row: PgRow) -> Result<Workspace, StorageError> {
    decode_workspace(&row)
}

fn decode_workspace<R: Row>(row: &R) -> Result<Workspace, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
    i64: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
{
    let state: String = row.try_get("state")?;
    let template_id: Option<String> = row.try_get("template_id")?;
    let yaml: String = row.try_get("template_snapshot_yaml")?;
    let document =
        WorkspaceTemplateDocument::parse(&yaml).map_err(|_| StorageError::InvalidWorkspace)?;
    Ok(Workspace {
        id: Uuid::parse_str(&row.try_get::<String, _>("id")?)?,
        short_id: row.try_get("short_id")?,
        organization_id: Uuid::parse_str(&row.try_get::<String, _>("organization_id")?)?,
        owner_id: Uuid::parse_str(&row.try_get::<String, _>("owner_id")?)?,
        name: row.try_get("name")?,
        template_id: template_id.map(|id| Uuid::parse_str(&id)).transpose()?,
        template: document.spec,
        state: WorkspaceState::from_database(&state)
            .ok_or(StorageError::UnknownWorkspaceState(state))?,
        generation: as_u64(row.try_get("generation")?)?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn as_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::InvalidWorkspace)
}
fn as_u64(value: i64) -> Result<u64, StorageError> {
    u64::try_from(value).map_err(|_| StorageError::InvalidWorkspace)
}
