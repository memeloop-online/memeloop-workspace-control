use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, SqliteConnection, postgres::PgRow, sqlite::SqliteRow};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec};

use super::{Database, StorageError, image_policy_store::validate_image};

mod row;

use row::{TemplateDatabaseRow, decode_optional_template, decode_template};

const TEMPLATE_COLUMNS: &str =
    "id, organization_id, template_yaml, enabled, created_at, updated_at";

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
    allow_cluster_access: bool,
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
    decode_snapshot(yaml, allow_cluster_access)
}

pub(super) async fn resolve_template_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    template_id: Uuid,
    organization_id: Uuid,
    allow_cluster_access: bool,
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
    decode_snapshot(yaml, allow_cluster_access)
}

fn decode_snapshot(
    yaml: Option<String>,
    allow_cluster_access: bool,
) -> Result<ResolvedTemplateSnapshot, StorageError> {
    let yaml = yaml.ok_or(StorageError::TemplateNotFound)?;
    let document = parse_document(&yaml)?;
    ensure_cluster_access(&document.spec, allow_cluster_access)?;
    Ok(ResolvedTemplateSnapshot {
        yaml,
        spec: document.spec,
    })
}

impl Database {
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
        allow_cluster_access: bool,
        now: i64,
    ) -> Result<WorkspaceTemplate, StorageError> {
        let yaml = normalize_yaml(&command.yaml)?;
        let document = parse_document(&yaml)?;
        ensure_cluster_access(&document.spec, allow_cluster_access)?;
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
        allow_cluster_access: bool,
        now: i64,
    ) -> Result<WorkspaceTemplate, StorageError> {
        let yaml = normalize_yaml(yaml)?;
        let document = parse_document(&yaml)?;
        ensure_cluster_access(&document.spec, allow_cluster_access)?;
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
        allow_cluster_access: bool,
        now: i64,
    ) -> Result<WorkspaceTemplate, StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                let current = sqlx::query(&format!("SELECT {TEMPLATE_COLUMNS} FROM workspace_templates WHERE installation_id = ?1 AND id = ?2"))
                    .bind(installation_id.as_str()).bind(template_id.to_string()).fetch_optional(&mut *transaction).await?.map(TemplateDatabaseRow::Sqlite);
                ensure_template_access(current, allow_cluster_access)?;
                let row = sqlx::query(&format!("UPDATE workspace_templates SET enabled = ?1, updated_at = ?2 WHERE installation_id = ?3 AND id = ?4 RETURNING {TEMPLATE_COLUMNS}"))
                    .bind(i64::from(enabled)).bind(now).bind(installation_id.as_str()).bind(template_id.to_string()).fetch_optional(&mut *transaction).await?.map(TemplateDatabaseRow::Sqlite);
                transaction.commit().await?;
                decode_optional_template(row)
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                let current = sqlx::query(&format!("SELECT {TEMPLATE_COLUMNS} FROM workspace_templates WHERE installation_id = $1 AND id = $2 FOR UPDATE"))
                    .bind(installation_id.as_str()).bind(template_id.to_string()).fetch_optional(&mut *transaction).await?.map(TemplateDatabaseRow::Postgres);
                ensure_template_access(current, allow_cluster_access)?;
                let row = sqlx::query(&format!("UPDATE workspace_templates SET enabled = $1, updated_at = $2 WHERE installation_id = $3 AND id = $4 RETURNING {TEMPLATE_COLUMNS}"))
                    .bind(i64::from(enabled)).bind(now).bind(installation_id.as_str()).bind(template_id.to_string()).fetch_optional(&mut *transaction).await?.map(TemplateDatabaseRow::Postgres);
                transaction.commit().await?;
                decode_optional_template(row)
            }
        }
    }

    pub async fn delete_workspace_template(
        &self,
        template_id: Uuid,
        allow_cluster_access: bool,
    ) -> Result<WorkspaceTemplate, StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                let current = sqlx::query(&format!("SELECT {TEMPLATE_COLUMNS} FROM workspace_templates WHERE installation_id = ?1 AND id = ?2"))
                    .bind(installation_id.as_str()).bind(template_id.to_string()).fetch_optional(&mut *transaction).await?.map(TemplateDatabaseRow::Sqlite);
                let template = ensure_template_access(current, allow_cluster_access)?;
                let referenced = sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM workspaces WHERE installation_id = ?1 AND template_id = ?2)")
                    .bind(installation_id.as_str()).bind(template_id.to_string()).fetch_one(&mut *transaction).await? != 0;
                ensure_template_deletable(template.enabled, referenced)?;
                sqlx::query(
                    "DELETE FROM workspace_templates WHERE installation_id = ?1 AND id = ?2",
                )
                .bind(installation_id.as_str())
                .bind(template_id.to_string())
                .execute(&mut *transaction)
                .await?;
                transaction.commit().await?;
                Ok(template)
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                let current = sqlx::query(&format!("SELECT {TEMPLATE_COLUMNS} FROM workspace_templates WHERE installation_id = $1 AND id = $2 FOR UPDATE"))
                    .bind(installation_id.as_str()).bind(template_id.to_string()).fetch_optional(&mut *transaction).await?.map(TemplateDatabaseRow::Postgres);
                let template = ensure_template_access(current, allow_cluster_access)?;
                let referenced = sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM workspaces WHERE installation_id = $1 AND template_id = $2)")
                    .bind(installation_id.as_str()).bind(template_id.to_string()).fetch_one(&mut *transaction).await?;
                ensure_template_deletable(template.enabled, referenced)?;
                sqlx::query(
                    "DELETE FROM workspace_templates WHERE installation_id = $1 AND id = $2",
                )
                .bind(installation_id.as_str())
                .bind(template_id.to_string())
                .execute(&mut *transaction)
                .await?;
                transaction.commit().await?;
                Ok(template)
            }
        }
    }
}

fn ensure_template_access(
    row: Option<TemplateDatabaseRow>,
    allow_cluster_access: bool,
) -> Result<WorkspaceTemplate, StorageError> {
    let template = match row {
        Some(TemplateDatabaseRow::Sqlite(row)) => decode_template(row),
        Some(TemplateDatabaseRow::Postgres(row)) => decode_template(row),
        None => return Err(StorageError::TemplateNotFound),
    }?;
    ensure_cluster_access(&template.template, allow_cluster_access)?;
    Ok(template)
}

fn ensure_template_deletable(enabled: bool, referenced: bool) -> Result<(), StorageError> {
    if enabled {
        return Err(StorageError::TemplateMustBeDisabled);
    }
    if referenced {
        return Err(StorageError::TemplateInUse);
    }
    Ok(())
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

fn ensure_cluster_access(
    spec: &WorkspaceTemplateSpec,
    allow_cluster_access: bool,
) -> Result<(), StorageError> {
    if spec.cluster_access && !allow_cluster_access {
        return Err(StorageError::PrivilegedTemplateForbidden);
    }
    Ok(())
}

fn as_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::InvalidTemplate)
}
