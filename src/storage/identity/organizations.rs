use uuid::Uuid;

use crate::quota::Resources;
use crate::storage::{Database, StorageError};

use super::{CreateOrganization, Organization, as_i64, backend};

impl Database {
    pub async fn create_organization(
        &self,
        command: CreateOrganization,
        now: i64,
    ) -> Result<Organization, StorageError> {
        let organization = Organization {
            id: Uuid::now_v7(),
            name: command.name.trim().to_owned(),
            created_at: now,
        };
        if organization.name.is_empty() {
            return Err(StorageError::OrganizationNotFound);
        }
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                backend::insert_organization_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    &organization,
                    command.owner_user_id,
                    now,
                )
                .await?;
                transaction.commit().await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                backend::insert_organization_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    &organization,
                    command.owner_user_id,
                    now,
                )
                .await?;
                transaction.commit().await?;
            }
        }
        Ok(organization)
    }

    pub async fn set_organization_quota(
        &self,
        organization_id: Uuid,
        resources: Resources,
        now: i64,
    ) -> Result<(), StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query(
                "INSERT INTO organization_quotas (installation_id, organization_id, cpu_millis, \
                memory_mib, gpu_count, disk_gib, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                ON CONFLICT (installation_id, organization_id) DO UPDATE SET cpu_millis = excluded.cpu_millis, \
                memory_mib = excluded.memory_mib, gpu_count = excluded.gpu_count, \
                disk_gib = excluded.disk_gib, updated_at = excluded.updated_at",
            )
            .bind(installation_id.as_str()).bind(organization_id.to_string())
            .bind(as_i64(resources.cpu_millis)?).bind(as_i64(resources.memory_mib)?)
            .bind(i64::from(resources.gpu_count)).bind(as_i64(resources.disk_gib)?).bind(now)
            .execute(pool).await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query(
                "INSERT INTO organization_quotas (installation_id, organization_id, cpu_millis, \
                memory_mib, gpu_count, disk_gib, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7) \
                ON CONFLICT (installation_id, organization_id) DO UPDATE SET cpu_millis = excluded.cpu_millis, \
                memory_mib = excluded.memory_mib, gpu_count = excluded.gpu_count, \
                disk_gib = excluded.disk_gib, updated_at = excluded.updated_at",
            )
            .bind(installation_id.as_str()).bind(organization_id.to_string())
            .bind(as_i64(resources.cpu_millis)?).bind(as_i64(resources.memory_mib)?)
            .bind(i64::from(resources.gpu_count)).bind(as_i64(resources.disk_gib)?).bind(now)
            .execute(pool).await?;
            }
        };
        Ok(())
    }
}
