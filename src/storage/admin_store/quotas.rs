use sqlx::Row;
use uuid::Uuid;

use crate::quota::Resources;
use crate::storage::{Database, StorageError};

impl Database {
    pub async fn get_organization_quota(
        &self,
        organization_id: Uuid,
    ) -> Result<Option<Resources>, StorageError> {
        let row = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let row = sqlx::query("SELECT cpu_millis, memory_mib, gpu_count, disk_gib FROM organization_quotas WHERE installation_id = ?1 AND organization_id = ?2")
                    .bind(installation_id.as_str())
                    .bind(organization_id.to_string())
                    .fetch_optional(pool)
                    .await?;
                return row.map(decode_resources).transpose();
            }
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query("SELECT cpu_millis, memory_mib, gpu_count, disk_gib FROM organization_quotas WHERE installation_id = $1 AND organization_id = $2")
                .bind(installation_id.as_str())
                .bind(organization_id.to_string())
                .fetch_optional(pool)
                .await?,
        };
        row.map(decode_resources).transpose()
    }

    pub async fn set_user_quota(
        &self,
        user_id: Uuid,
        resources: Resources,
        now: i64,
    ) -> Result<(), StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO user_quotas (installation_id, user_id, cpu_millis, memory_mib, gpu_count, disk_gib, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT (installation_id, user_id) DO UPDATE SET cpu_millis = excluded.cpu_millis, memory_mib = excluded.memory_mib, gpu_count = excluded.gpu_count, disk_gib = excluded.disk_gib, updated_at = excluded.updated_at")
                    .bind(installation_id.as_str())
                    .bind(user_id.to_string())
                    .bind(as_i64(resources.cpu_millis)?)
                    .bind(as_i64(resources.memory_mib)?)
                    .bind(i64::from(resources.gpu_count))
                    .bind(as_i64(resources.disk_gib)?)
                    .bind(now)
                    .execute(pool)
                    .await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO user_quotas (installation_id, user_id, cpu_millis, memory_mib, gpu_count, disk_gib, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (installation_id, user_id) DO UPDATE SET cpu_millis = excluded.cpu_millis, memory_mib = excluded.memory_mib, gpu_count = excluded.gpu_count, disk_gib = excluded.disk_gib, updated_at = excluded.updated_at")
                    .bind(installation_id.as_str())
                    .bind(user_id.to_string())
                    .bind(as_i64(resources.cpu_millis)?)
                    .bind(as_i64(resources.memory_mib)?)
                    .bind(i64::from(resources.gpu_count))
                    .bind(as_i64(resources.disk_gib)?)
                    .bind(now)
                    .execute(pool)
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn get_user_quota(&self, user_id: Uuid) -> Result<Option<Resources>, StorageError> {
        let row = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let row = sqlx::query("SELECT cpu_millis, memory_mib, gpu_count, disk_gib FROM user_quotas WHERE installation_id = ?1 AND user_id = ?2")
                    .bind(installation_id.as_str())
                    .bind(user_id.to_string())
                    .fetch_optional(pool)
                    .await?;
                return row.map(decode_resources).transpose();
            }
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query("SELECT cpu_millis, memory_mib, gpu_count, disk_gib FROM user_quotas WHERE installation_id = $1 AND user_id = $2")
                .bind(installation_id.as_str())
                .bind(user_id.to_string())
                .fetch_optional(pool)
                .await?,
        };
        row.map(decode_resources).transpose()
    }
}

fn decode_resources<R: Row>(row: R) -> Result<Resources, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    Ok(Resources {
        cpu_millis: u64::try_from(row.try_get::<i64, _>("cpu_millis")?)
            .map_err(|_| StorageError::InvalidWorkspace)?,
        memory_mib: u64::try_from(row.try_get::<i64, _>("memory_mib")?)
            .map_err(|_| StorageError::InvalidWorkspace)?,
        gpu_count: u32::try_from(row.try_get::<i64, _>("gpu_count")?)
            .map_err(|_| StorageError::InvalidWorkspace)?,
        disk_gib: u64::try_from(row.try_get::<i64, _>("disk_gib")?)
            .map_err(|_| StorageError::InvalidWorkspace)?,
    })
}

fn as_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::InvalidWorkspace)
}
