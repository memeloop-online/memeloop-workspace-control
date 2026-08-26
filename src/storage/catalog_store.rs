use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, Row, SqliteConnection, postgres::PgRow, sqlite::SqliteRow};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{quota::Resources, workspaces::AccessMode};

use super::{Database, StorageError};

pub const IMAGE_CONTRACT_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImagePolicy {
    pub image: String,
    pub contract_version: u16,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

pub(super) async fn admit_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace: &crate::workspaces::Workspace,
) -> Result<(), StorageError> {
    let allowed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM image_policies WHERE installation_id = ?1 AND image = ?2 AND enabled = 1 AND contract_version = ?3")
        .bind(installation_id).bind(&workspace.image).bind(i64::from(IMAGE_CONTRACT_VERSION)).fetch_one(&mut *connection).await?;
    if allowed != 1 {
        return Err(StorageError::ImageNotAllowed);
    }
    if let Some(template_id) = workspace.template_id {
        let matched: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workspace_templates WHERE installation_id = ?1 AND id = ?2 AND enabled = 1 AND (organization_id IS NULL OR organization_id = ?3) AND image = ?4 AND access_mode = ?5 AND cpu_millis = ?6 AND memory_mib = ?7 AND gpu_count = ?8 AND disk_gib = ?9")
            .bind(installation_id).bind(template_id.to_string()).bind(workspace.organization_id.to_string()).bind(&workspace.image).bind(workspace.access_mode.as_str()).bind(as_i64(workspace.resources.cpu_millis)?).bind(as_i64(workspace.resources.memory_mib)?).bind(i64::from(workspace.resources.gpu_count)).bind(as_i64(workspace.resources.disk_gib)?).fetch_one(&mut *connection).await?;
        if matched != 1 {
            return Err(StorageError::TemplateNotFound);
        }
    }
    Ok(())
}

pub(super) async fn admit_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &crate::workspaces::Workspace,
) -> Result<(), StorageError> {
    let allowed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM image_policies WHERE installation_id = $1 AND image = $2 AND enabled = 1 AND contract_version = $3")
        .bind(installation_id).bind(&workspace.image).bind(i64::from(IMAGE_CONTRACT_VERSION)).fetch_one(&mut *connection).await?;
    if allowed != 1 {
        return Err(StorageError::ImageNotAllowed);
    }
    if let Some(template_id) = workspace.template_id {
        let matched: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workspace_templates WHERE installation_id = $1 AND id = $2 AND enabled = 1 AND (organization_id IS NULL OR organization_id = $3) AND image = $4 AND access_mode = $5 AND cpu_millis = $6 AND memory_mib = $7 AND gpu_count = $8 AND disk_gib = $9")
            .bind(installation_id).bind(template_id.to_string()).bind(workspace.organization_id.to_string()).bind(&workspace.image).bind(workspace.access_mode.as_str()).bind(as_i64(workspace.resources.cpu_millis)?).bind(as_i64(workspace.resources.memory_mib)?).bind(i64::from(workspace.resources.gpu_count)).bind(as_i64(workspace.resources.disk_gib)?).fetch_one(&mut *connection).await?;
        if matched != 1 {
            return Err(StorageError::TemplateNotFound);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WorkspaceTemplate {
    pub id: Uuid,
    pub organization_id: Option<Uuid>,
    pub name: String,
    pub image: String,
    pub access_mode: AccessMode,
    pub resources: Resources,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateWorkspaceTemplate {
    pub organization_id: Option<Uuid>,
    pub name: String,
    pub image: String,
    pub access_mode: AccessMode,
    pub resources: Resources,
}

impl Database {
    pub async fn list_image_policies(&self) -> Result<Vec<ImagePolicy>, StorageError> {
        match self {
            Self::Sqlite { pool, installation_id } => sqlx::query("SELECT image, contract_version, enabled, created_at, updated_at FROM image_policies WHERE installation_id = ?1 ORDER BY image")
                .bind(installation_id.as_str()).fetch_all(pool).await?.into_iter().map(decode_image).collect(),
            Self::Postgres { pool, installation_id } => sqlx::query("SELECT image, contract_version, enabled, created_at, updated_at FROM image_policies WHERE installation_id = $1 ORDER BY image")
                .bind(installation_id.as_str()).fetch_all(pool).await?.into_iter().map(decode_image).collect(),
        }
    }

    pub async fn upsert_image_policy(
        &self,
        image: &str,
        enabled: bool,
        now: i64,
    ) -> Result<ImagePolicy, StorageError> {
        validate_image(image)?;
        let image = image.trim();
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO image_policies (installation_id, image, contract_version, enabled, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5) ON CONFLICT (installation_id, image) DO UPDATE SET contract_version = excluded.contract_version, enabled = excluded.enabled, updated_at = excluded.updated_at")
                .bind(installation_id.as_str()).bind(image).bind(i64::from(IMAGE_CONTRACT_VERSION)).bind(i64::from(enabled)).bind(now).execute(pool).await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO image_policies (installation_id, image, contract_version, enabled, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $5) ON CONFLICT (installation_id, image) DO UPDATE SET contract_version = excluded.contract_version, enabled = excluded.enabled, updated_at = excluded.updated_at")
                .bind(installation_id.as_str()).bind(image).bind(i64::from(IMAGE_CONTRACT_VERSION)).bind(i64::from(enabled)).bind(now).execute(pool).await?;
            }
        };
        Ok(ImagePolicy {
            image: image.to_owned(),
            contract_version: IMAGE_CONTRACT_VERSION,
            enabled,
            created_at: now,
            updated_at: now,
        })
    }

    pub async fn list_workspace_templates(
        &self,
        organization_id: Option<Uuid>,
    ) -> Result<Vec<WorkspaceTemplate>, StorageError> {
        let organization = organization_id.map(|id| id.to_string());
        match self {
            Self::Sqlite { pool, installation_id } => sqlx::query("SELECT id, organization_id, name, image, access_mode, cpu_millis, memory_mib, gpu_count, disk_gib, enabled, created_at, updated_at FROM workspace_templates WHERE installation_id = ?1 AND (organization_id IS NULL OR organization_id = ?2) ORDER BY organization_id, name")
                .bind(installation_id.as_str()).bind(organization).fetch_all(pool).await?.into_iter().map(decode_template).collect(),
            Self::Postgres { pool, installation_id } => sqlx::query("SELECT id, organization_id, name, image, access_mode, cpu_millis, memory_mib, gpu_count, disk_gib, enabled, created_at, updated_at FROM workspace_templates WHERE installation_id = $1 AND (organization_id IS NULL OR organization_id = $2) ORDER BY organization_id, name")
                .bind(installation_id.as_str()).bind(organization).fetch_all(pool).await?.into_iter().map(decode_template).collect(),
        }
    }

    pub async fn create_workspace_template(
        &self,
        command: CreateWorkspaceTemplate,
        now: i64,
    ) -> Result<WorkspaceTemplate, StorageError> {
        if command.name.trim().is_empty()
            || command.name.len() > 120
            || !command.resources.valid_workspace_request()
        {
            return Err(StorageError::InvalidTemplate);
        }
        validate_image(&command.image)?;
        let template = WorkspaceTemplate {
            id: Uuid::now_v7(),
            organization_id: command.organization_id,
            name: command.name.trim().to_owned(),
            image: command.image.trim().to_owned(),
            access_mode: command.access_mode,
            resources: command.resources,
            enabled: true,
            created_at: now,
            updated_at: now,
        };
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO workspace_templates (id, installation_id, organization_id, name, image, access_mode, cpu_millis, memory_mib, gpu_count, disk_gib, enabled, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11, ?11)")
                .bind(template.id.to_string()).bind(installation_id.as_str()).bind(template.organization_id.map(|id| id.to_string())).bind(&template.name).bind(&template.image).bind(template.access_mode.as_str()).bind(as_i64(template.resources.cpu_millis)?).bind(as_i64(template.resources.memory_mib)?).bind(i64::from(template.resources.gpu_count)).bind(as_i64(template.resources.disk_gib)?).bind(now).execute(pool).await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO workspace_templates (id, installation_id, organization_id, name, image, access_mode, cpu_millis, memory_mib, gpu_count, disk_gib, enabled, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 1, $11, $11)")
                .bind(template.id.to_string()).bind(installation_id.as_str()).bind(template.organization_id.map(|id| id.to_string())).bind(&template.name).bind(&template.image).bind(template.access_mode.as_str()).bind(as_i64(template.resources.cpu_millis)?).bind(as_i64(template.resources.memory_mib)?).bind(i64::from(template.resources.gpu_count)).bind(as_i64(template.resources.disk_gib)?).bind(now).execute(pool).await?;
            }
        };
        Ok(template)
    }
}

fn validate_image(image: &str) -> Result<(), StorageError> {
    let image = image.trim();
    if image.is_empty() || image.len() > 512 || image.chars().any(char::is_whitespace) {
        return Err(StorageError::InvalidTemplate);
    }
    Ok(())
}

fn decode_image<R: Row>(row: R) -> Result<ImagePolicy, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    Ok(ImagePolicy {
        image: row.try_get("image")?,
        contract_version: u16::try_from(row.try_get::<i64, _>("contract_version")?)
            .map_err(|_| StorageError::InvalidTemplate)?,
        enabled: row.try_get::<i64, _>("enabled")? != 0,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn decode_template(row: impl TemplateRow) -> Result<WorkspaceTemplate, StorageError> {
    row.decode()
}

trait TemplateRow {
    fn decode(self) -> Result<WorkspaceTemplate, StorageError>;
}
impl TemplateRow for SqliteRow {
    fn decode(self) -> Result<WorkspaceTemplate, StorageError> {
        decode_template_row(&self)
    }
}
impl TemplateRow for PgRow {
    fn decode(self) -> Result<WorkspaceTemplate, StorageError> {
        decode_template_row(&self)
    }
}

fn decode_template_row<R: Row>(row: &R) -> Result<WorkspaceTemplate, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    let organization: Option<String> = row.try_get("organization_id")?;
    let access: String = row.try_get("access_mode")?;
    Ok(WorkspaceTemplate {
        id: Uuid::parse_str(&row.try_get::<String, _>("id")?)?,
        organization_id: organization.map(|id| Uuid::parse_str(&id)).transpose()?,
        name: row.try_get("name")?,
        image: row.try_get("image")?,
        access_mode: AccessMode::from_database(&access)
            .ok_or(StorageError::UnknownAccessMode(access))?,
        resources: Resources {
            cpu_millis: as_u64(row.try_get("cpu_millis")?)?,
            memory_mib: as_u64(row.try_get("memory_mib")?)?,
            gpu_count: u32::try_from(row.try_get::<i64, _>("gpu_count")?)
                .map_err(|_| StorageError::InvalidTemplate)?,
            disk_gib: as_u64(row.try_get("disk_gib")?)?,
        },
        enabled: row.try_get::<i64, _>("enabled")? != 0,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn as_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::InvalidTemplate)
}
fn as_u64(value: i64) -> Result<u64, StorageError> {
    u64::try_from(value).map_err(|_| StorageError::InvalidTemplate)
}
