use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, Row, SqliteConnection};
use utoipa::ToSchema;

use crate::workspaces::Workspace;

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
    workspace: &Workspace,
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
    workspace: &Workspace,
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
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query("SELECT image, contract_version, enabled, created_at, updated_at FROM image_policies WHERE installation_id = ?1 ORDER BY image")
                .bind(installation_id.as_str())
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(decode_image)
                .collect(),
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query("SELECT image, contract_version, enabled, created_at, updated_at FROM image_policies WHERE installation_id = $1 ORDER BY image")
                .bind(installation_id.as_str())
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(decode_image)
                .collect(),
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
}

pub(super) fn validate_image(image: &str) -> Result<(), StorageError> {
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
