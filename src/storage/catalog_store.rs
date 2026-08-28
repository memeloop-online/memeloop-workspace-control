use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, Row, SqliteConnection, postgres::PgRow, sqlite::SqliteRow};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    quota::Resources,
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::AccessMode,
};

use super::{Database, StorageError};

pub const IMAGE_CONTRACT_VERSION: u16 = 1;
const TEMPLATE_COLUMNS: &str =
    "id, organization_id, template_yaml, enabled, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImagePolicy {
    pub image: String,
    pub contract_version: u16,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WorkspaceTemplate {
    pub id: Uuid,
    pub organization_id: Option<Uuid>,
    pub name: String,
    #[serde(flatten)]
    pub template: WorkspaceTemplateSpec,
    pub yaml: String,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateWorkspaceTemplate {
    pub organization_id: Option<Uuid>,
    pub yaml: String,
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedTemplateSnapshot {
    pub yaml: String,
    pub spec: WorkspaceTemplateSpec,
}

pub(super) async fn resolve_template_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    template_id: Uuid,
    organization_id: Uuid,
) -> Result<ResolvedTemplateSnapshot, StorageError> {
    let yaml: Option<String> = sqlx::query_scalar(
        "SELECT template_yaml FROM workspace_templates WHERE installation_id = ?1 AND id = ?2 \
         AND enabled = 1 AND (organization_id IS NULL OR organization_id = ?3)",
    )
    .bind(installation_id)
    .bind(template_id.to_string())
    .bind(organization_id.to_string())
    .fetch_optional(&mut *connection)
    .await?;
    decode_snapshot(yaml)
}

pub(super) async fn resolve_template_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    template_id: Uuid,
    organization_id: Uuid,
) -> Result<ResolvedTemplateSnapshot, StorageError> {
    let yaml: Option<String> = sqlx::query_scalar(
        "SELECT template_yaml FROM workspace_templates WHERE installation_id = $1 AND id = $2 \
         AND enabled = 1 AND (organization_id IS NULL OR organization_id = $3) FOR SHARE",
    )
    .bind(installation_id)
    .bind(template_id.to_string())
    .bind(organization_id.to_string())
    .fetch_optional(&mut *connection)
    .await?;
    decode_snapshot(yaml)
}

fn decode_snapshot(yaml: Option<String>) -> Result<ResolvedTemplateSnapshot, StorageError> {
    let yaml = yaml.ok_or(StorageError::TemplateNotFound)?;
    let document = parse_document(&yaml)?;
    Ok(ResolvedTemplateSnapshot {
        yaml,
        spec: document.spec,
    })
}

pub(super) async fn admit_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace: &crate::workspaces::Workspace,
) -> Result<(), StorageError> {
    let allowed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM image_policies WHERE installation_id = ?1 AND image = ?2 AND enabled = 1 AND contract_version = ?3")
        .bind(installation_id).bind(&workspace.template.image).bind(i64::from(IMAGE_CONTRACT_VERSION)).fetch_one(&mut *connection).await?;
    (allowed == 1)
        .then_some(())
        .ok_or(StorageError::ImageNotAllowed)
}

pub(super) async fn admit_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &crate::workspaces::Workspace,
) -> Result<(), StorageError> {
    let allowed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM image_policies WHERE installation_id = $1 AND image = $2 AND enabled = 1 AND contract_version = $3")
        .bind(installation_id).bind(&workspace.template.image).bind(i64::from(IMAGE_CONTRACT_VERSION)).fetch_one(&mut *connection).await?;
    (allowed == 1)
        .then_some(())
        .ok_or(StorageError::ImageNotAllowed)
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
            Self::Sqlite { pool, installation_id } => sqlx::query(&format!("SELECT {TEMPLATE_COLUMNS} FROM workspace_templates WHERE installation_id = ?1 AND (organization_id IS NULL OR organization_id = ?2) ORDER BY organization_id, name"))
                .bind(installation_id.as_str()).bind(organization).fetch_all(pool).await?.into_iter().map(decode_template).collect(),
            Self::Postgres { pool, installation_id } => sqlx::query(&format!("SELECT {TEMPLATE_COLUMNS} FROM workspace_templates WHERE installation_id = $1 AND (organization_id IS NULL OR organization_id = $2) ORDER BY organization_id, name"))
                .bind(installation_id.as_str()).bind(organization).fetch_all(pool).await?.into_iter().map(decode_template).collect(),
        }
    }

    pub async fn get_workspace_template(
        &self,
        template_id: Uuid,
    ) -> Result<WorkspaceTemplate, StorageError> {
        let row = match self {
            Self::Sqlite { pool, installation_id } => sqlx::query(&format!("SELECT {TEMPLATE_COLUMNS} FROM workspace_templates WHERE installation_id = ?1 AND id = ?2"))
                .bind(installation_id.as_str()).bind(template_id.to_string()).fetch_optional(pool).await?.map(TemplateDatabaseRow::Sqlite),
            Self::Postgres { pool, installation_id } => sqlx::query(&format!("SELECT {TEMPLATE_COLUMNS} FROM workspace_templates WHERE installation_id = $1 AND id = $2"))
                .bind(installation_id.as_str()).bind(template_id.to_string()).fetch_optional(pool).await?.map(TemplateDatabaseRow::Postgres),
        };
        decode_optional_template(row)
    }

    pub async fn create_workspace_template(
        &self,
        command: CreateWorkspaceTemplate,
        now: i64,
    ) -> Result<WorkspaceTemplate, StorageError> {
        let yaml = normalize_yaml(&command.yaml)?;
        let document = parse_document(&yaml)?;
        validate_image(&document.spec.image)?;
        let template = WorkspaceTemplate {
            id: Uuid::now_v7(),
            organization_id: command.organization_id,
            name: document.metadata.name,
            template: document.spec,
            yaml,
            enabled: true,
            created_at: now,
            updated_at: now,
        };
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => insert_template_sqlite(pool, installation_id.as_str(), &template, now).await?,
            Self::Postgres {
                pool,
                installation_id,
            } => insert_template_postgres(pool, installation_id.as_str(), &template, now).await?,
        }
        Ok(template)
    }

    pub async fn replace_workspace_template(
        &self,
        template_id: Uuid,
        yaml: &str,
        now: i64,
    ) -> Result<WorkspaceTemplate, StorageError> {
        let yaml = normalize_yaml(yaml)?;
        let document = parse_document(&yaml)?;
        validate_image(&document.spec.image)?;
        let row = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => update_template_sqlite(
                pool,
                installation_id.as_str(),
                template_id,
                &document,
                &yaml,
                now,
            )
            .await?
            .map(TemplateDatabaseRow::Sqlite),
            Self::Postgres {
                pool,
                installation_id,
            } => update_template_postgres(
                pool,
                installation_id.as_str(),
                template_id,
                &document,
                &yaml,
                now,
            )
            .await?
            .map(TemplateDatabaseRow::Postgres),
        };
        decode_optional_template(row)
    }

    pub async fn set_workspace_template_enabled(
        &self,
        template_id: Uuid,
        enabled: bool,
        now: i64,
    ) -> Result<WorkspaceTemplate, StorageError> {
        let row = match self {
            Self::Sqlite { pool, installation_id } => sqlx::query(&format!("UPDATE workspace_templates SET enabled = ?1, updated_at = ?2 WHERE installation_id = ?3 AND id = ?4 RETURNING {TEMPLATE_COLUMNS}"))
                .bind(i64::from(enabled)).bind(now).bind(installation_id.as_str()).bind(template_id.to_string()).fetch_optional(pool).await?.map(TemplateDatabaseRow::Sqlite),
            Self::Postgres { pool, installation_id } => sqlx::query(&format!("UPDATE workspace_templates SET enabled = $1, updated_at = $2 WHERE installation_id = $3 AND id = $4 RETURNING {TEMPLATE_COLUMNS}"))
                .bind(i64::from(enabled)).bind(now).bind(installation_id.as_str()).bind(template_id.to_string()).fetch_optional(pool).await?.map(TemplateDatabaseRow::Postgres),
        };
        decode_optional_template(row)
    }

    pub(super) async fn backfill_template_yaml(&self) -> Result<(), StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                for row in sqlx::query("SELECT id, name, runtime_profile, image, access_mode, cpu_millis, memory_mib, gpu_count, disk_gib FROM workspace_templates WHERE installation_id = ?1 AND template_yaml = ''").bind(installation_id.as_str()).fetch_all(pool).await? {
                    let (id, yaml) = legacy_yaml(&row)?;
                    sqlx::query("UPDATE workspace_templates SET template_yaml = ?1 WHERE installation_id = ?2 AND id = ?3 AND template_yaml = ''").bind(yaml).bind(installation_id.as_str()).bind(id).execute(pool).await?;
                }
                for row in sqlx::query("SELECT id, name, runtime_profile, image, access_mode, cpu_millis, memory_mib, gpu_count, disk_gib FROM workspaces WHERE installation_id = ?1 AND template_snapshot_yaml = ''").bind(installation_id.as_str()).fetch_all(pool).await? {
                    let (id, yaml) = legacy_yaml(&row)?;
                    sqlx::query("UPDATE workspaces SET template_snapshot_yaml = ?1 WHERE installation_id = ?2 AND id = ?3 AND template_snapshot_yaml = ''").bind(yaml).bind(installation_id.as_str()).bind(id).execute(pool).await?;
                }
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                for row in sqlx::query("SELECT id, name, runtime_profile, image, access_mode, cpu_millis, memory_mib, gpu_count, disk_gib FROM workspace_templates WHERE installation_id = $1 AND template_yaml = ''").bind(installation_id.as_str()).fetch_all(pool).await? {
                    let (id, yaml) = legacy_yaml(&row)?;
                    sqlx::query("UPDATE workspace_templates SET template_yaml = $1 WHERE installation_id = $2 AND id = $3 AND template_yaml = ''").bind(yaml).bind(installation_id.as_str()).bind(id).execute(pool).await?;
                }
                for row in sqlx::query("SELECT id, name, runtime_profile, image, access_mode, cpu_millis, memory_mib, gpu_count, disk_gib FROM workspaces WHERE installation_id = $1 AND template_snapshot_yaml = ''").bind(installation_id.as_str()).fetch_all(pool).await? {
                    let (id, yaml) = legacy_yaml(&row)?;
                    sqlx::query("UPDATE workspaces SET template_snapshot_yaml = $1 WHERE installation_id = $2 AND id = $3 AND template_snapshot_yaml = ''").bind(yaml).bind(installation_id.as_str()).bind(id).execute(pool).await?;
                }
            }
        }
        Ok(())
    }
}

async fn insert_template_sqlite(
    pool: &sqlx::SqlitePool,
    installation_id: &str,
    template: &WorkspaceTemplate,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO workspace_templates (id, installation_id, organization_id, name, image, access_mode, cpu_millis, memory_mib, gpu_count, disk_gib, enabled, created_at, updated_at, template_yaml) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11, ?11, ?12)")
        .bind(template.id.to_string()).bind(installation_id).bind(template.organization_id.map(|id| id.to_string())).bind(&template.name).bind(&template.template.image).bind(template.template.access_mode.as_str()).bind(as_i64(template.template.resources.cpu_millis)?).bind(as_i64(template.template.resources.memory_mib)?).bind(i64::from(template.template.resources.gpu_count)).bind(as_i64(template.template.resources.disk_gib)?).bind(now).bind(&template.yaml).execute(pool).await?;
    Ok(())
}

async fn insert_template_postgres(
    pool: &sqlx::PgPool,
    installation_id: &str,
    template: &WorkspaceTemplate,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO workspace_templates (id, installation_id, organization_id, name, image, access_mode, cpu_millis, memory_mib, gpu_count, disk_gib, enabled, created_at, updated_at, template_yaml) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 1, $11, $11, $12)")
        .bind(template.id.to_string()).bind(installation_id).bind(template.organization_id.map(|id| id.to_string())).bind(&template.name).bind(&template.template.image).bind(template.template.access_mode.as_str()).bind(as_i64(template.template.resources.cpu_millis)?).bind(as_i64(template.template.resources.memory_mib)?).bind(i64::from(template.template.resources.gpu_count)).bind(as_i64(template.template.resources.disk_gib)?).bind(now).bind(&template.yaml).execute(pool).await?;
    Ok(())
}

async fn update_template_sqlite(
    pool: &sqlx::SqlitePool,
    installation_id: &str,
    id: Uuid,
    document: &WorkspaceTemplateDocument,
    yaml: &str,
    now: i64,
) -> Result<Option<SqliteRow>, StorageError> {
    Ok(sqlx::query(&format!("UPDATE workspace_templates SET name=?1,image=?2,access_mode=?3,cpu_millis=?4,memory_mib=?5,gpu_count=?6,disk_gib=?7,template_yaml=?8,updated_at=?9 WHERE installation_id=?10 AND id=?11 RETURNING {TEMPLATE_COLUMNS}"))
        .bind(document.metadata.name.trim()).bind(document.spec.image.trim()).bind(document.spec.access_mode.as_str()).bind(as_i64(document.spec.resources.cpu_millis)?).bind(as_i64(document.spec.resources.memory_mib)?).bind(i64::from(document.spec.resources.gpu_count)).bind(as_i64(document.spec.resources.disk_gib)?).bind(yaml).bind(now).bind(installation_id).bind(id.to_string()).fetch_optional(pool).await?)
}

async fn update_template_postgres(
    pool: &sqlx::PgPool,
    installation_id: &str,
    id: Uuid,
    document: &WorkspaceTemplateDocument,
    yaml: &str,
    now: i64,
) -> Result<Option<PgRow>, StorageError> {
    Ok(sqlx::query(&format!("UPDATE workspace_templates SET name=$1,image=$2,access_mode=$3,cpu_millis=$4,memory_mib=$5,gpu_count=$6,disk_gib=$7,template_yaml=$8,updated_at=$9 WHERE installation_id=$10 AND id=$11 RETURNING {TEMPLATE_COLUMNS}"))
        .bind(document.metadata.name.trim()).bind(document.spec.image.trim()).bind(document.spec.access_mode.as_str()).bind(as_i64(document.spec.resources.cpu_millis)?).bind(as_i64(document.spec.resources.memory_mib)?).bind(i64::from(document.spec.resources.gpu_count)).bind(as_i64(document.spec.resources.disk_gib)?).bind(yaml).bind(now).bind(installation_id).bind(id.to_string()).fetch_optional(pool).await?)
}

fn parse_document(yaml: &str) -> Result<WorkspaceTemplateDocument, StorageError> {
    WorkspaceTemplateDocument::parse(yaml).map_err(|_| StorageError::InvalidTemplate)
}

fn normalize_yaml(yaml: &str) -> Result<String, StorageError> {
    if yaml.trim().is_empty() || yaml.len() > 128 * 1024 {
        return Err(StorageError::InvalidTemplate);
    }
    Ok(if yaml.ends_with('\n') {
        yaml.to_owned()
    } else {
        format!("{yaml}\n")
    })
}

fn legacy_yaml<R: Row>(row: &R) -> Result<(String, String), StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    let access = row.try_get::<String, _>("access_mode")?;
    let resources = Resources {
        cpu_millis: as_u64(row.try_get("cpu_millis")?)?,
        memory_mib: as_u64(row.try_get("memory_mib")?)?,
        gpu_count: u32::try_from(row.try_get::<i64, _>("gpu_count")?)
            .map_err(|_| StorageError::InvalidTemplate)?,
        disk_gib: as_u64(row.try_get("disk_gib")?)?,
    };
    let spec = WorkspaceTemplateSpec::from_legacy(
        &row.try_get::<String, _>("runtime_profile")?,
        row.try_get::<String, _>("image")?,
        AccessMode::from_database(&access).ok_or(StorageError::UnknownAccessMode(access))?,
        resources,
    )
    .map_err(|_| StorageError::InvalidTemplate)?;
    let document = WorkspaceTemplateDocument::new(row.try_get::<String, _>("name")?, spec);
    Ok((
        row.try_get("id")?,
        document
            .to_yaml()
            .map_err(|_| StorageError::InvalidTemplate)?,
    ))
}

fn decode_optional_template(
    row: Option<TemplateDatabaseRow>,
) -> Result<WorkspaceTemplate, StorageError> {
    match row {
        Some(TemplateDatabaseRow::Sqlite(row)) => decode_template(row),
        Some(TemplateDatabaseRow::Postgres(row)) => decode_template(row),
        None => Err(StorageError::TemplateNotFound),
    }
}

enum TemplateDatabaseRow {
    Sqlite(SqliteRow),
    Postgres(PgRow),
}

fn decode_template<R: Row>(row: R) -> Result<WorkspaceTemplate, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    let organization: Option<String> = row.try_get("organization_id")?;
    let yaml: String = row.try_get("template_yaml")?;
    let document = parse_document(&yaml)?;
    Ok(WorkspaceTemplate {
        id: Uuid::parse_str(&row.try_get::<String, _>("id")?)?,
        organization_id: organization.map(|id| Uuid::parse_str(&id)).transpose()?,
        name: document.metadata.name,
        template: document.spec,
        yaml,
        enabled: row.try_get::<i64, _>("enabled")? != 0,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
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

fn as_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::InvalidTemplate)
}
fn as_u64(value: i64) -> Result<u64, StorageError> {
    u64::try_from(value).map_err(|_| StorageError::InvalidTemplate)
}
