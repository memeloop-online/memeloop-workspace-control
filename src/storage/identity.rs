use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    auth::{Permission, Role, RoleBinding},
    quota::Resources,
};

use super::{
    ApiKeySummary, Database, StorageError,
    user_settings::{insert_key_postgres, insert_key_sqlite, token_prefix},
};

mod backend;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Principal {
    pub user_id: Uuid,
    pub display_name: String,
    pub system_admin: bool,
    pub memberships: Vec<Membership>,
}

impl Principal {
    pub fn allows(&self, permission: Permission, organization_id: Uuid) -> bool {
        if self.system_admin {
            return true;
        }
        self.memberships.iter().any(|membership| {
            RoleBinding {
                role: membership.role,
                organization_id: Some(membership.organization_id),
            }
            .allows(permission, organization_id)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Membership {
    pub organization_id: Uuid,
    pub role: Role,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Organization {
    pub id: Uuid,
    pub name: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateOrganization {
    pub name: String,
    pub owner_user_id: Uuid,
}

impl Database {
    pub async fn create_user(
        &self,
        display_name: &str,
        token: &str,
        system_admin: bool,
        now: i64,
    ) -> Result<Principal, StorageError> {
        validate_token(token)?;
        let user_id = Uuid::now_v7();
        let token_hash = hash_token(token);
        let initial_key = ApiKeySummary {
            id: user_id,
            name: "Initial key".to_owned(),
            prefix: token_prefix(token),
            last_used_at: None,
            created_at: now,
        };
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                sqlx::query(
                    "INSERT INTO users (id, installation_id, display_name, token_hash, \
                    system_admin, disabled, created_at) VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
                )
                .bind(user_id.to_string())
                .bind(installation_id.as_str())
                .bind(display_name)
                .bind(&token_hash)
                .bind(i64::from(system_admin))
                .bind(now)
                .execute(&mut *transaction)
                .await?;
                insert_key_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    user_id,
                    &initial_key,
                    &token_hash,
                )
                .await?;
                transaction.commit().await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                sqlx::query(
                    "INSERT INTO users (id, installation_id, display_name, token_hash, \
                    system_admin, disabled, created_at) VALUES ($1, $2, $3, $4, $5, 0, $6)",
                )
                .bind(user_id.to_string())
                .bind(installation_id.as_str())
                .bind(display_name)
                .bind(&token_hash)
                .bind(i64::from(system_admin))
                .bind(now)
                .execute(&mut *transaction)
                .await?;
                insert_key_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    user_id,
                    &initial_key,
                    &token_hash,
                )
                .await?;
                transaction.commit().await?;
            }
        }
        Ok(Principal {
            user_id,
            display_name: display_name.to_owned(),
            system_admin,
            memberships: Vec::new(),
        })
    }

    pub async fn authenticate(&self, token: &str) -> Result<Option<Principal>, StorageError> {
        if token.len() < 32 {
            return Ok(None);
        }
        let token_hash = hash_token(token);
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let row = sqlx::query(
                    "SELECT u.id, u.display_name, u.system_admin, k.id AS key_id, \
                    k.last_used_at FROM users u \
                    JOIN user_api_keys k ON k.installation_id = u.installation_id AND k.user_id = u.id \
                    WHERE u.installation_id = ?1 AND k.token_hash = ?2 AND k.revoked_at IS NULL AND u.disabled = 0",
                )
                .bind(installation_id.as_str())
                .bind(token_hash)
                .fetch_optional(pool)
                .await?;
                let Some(row) = row else { return Ok(None) };
                let user_id = Uuid::parse_str(row.try_get::<String, _>("id")?.as_str())?;
                let key_id: String = row.try_get("key_id")?;
                backend::mark_key_used_sqlite(
                    pool,
                    installation_id.as_str(),
                    &key_id,
                    row.try_get("last_used_at")?,
                )
                .await?;
                let memberships =
                    backend::sqlite_memberships(pool, installation_id.as_str(), user_id).await?;
                Ok(Some(Principal {
                    user_id,
                    display_name: row.try_get("display_name")?,
                    system_admin: row.try_get::<i64, _>("system_admin")? != 0,
                    memberships,
                }))
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let row = sqlx::query(
                    "SELECT u.id, u.display_name, u.system_admin, k.id AS key_id, \
                    k.last_used_at FROM users u \
                    JOIN user_api_keys k ON k.installation_id = u.installation_id AND k.user_id = u.id \
                    WHERE u.installation_id = $1 AND k.token_hash = $2 AND k.revoked_at IS NULL AND u.disabled = 0",
                )
                .bind(installation_id.as_str())
                .bind(token_hash)
                .fetch_optional(pool)
                .await?;
                let Some(row) = row else { return Ok(None) };
                let user_id = Uuid::parse_str(row.try_get::<String, _>("id")?.as_str())?;
                let key_id: String = row.try_get("key_id")?;
                backend::mark_key_used_postgres(
                    pool,
                    installation_id.as_str(),
                    &key_id,
                    row.try_get("last_used_at")?,
                )
                .await?;
                let memberships =
                    backend::postgres_memberships(pool, installation_id.as_str(), user_id).await?;
                Ok(Some(Principal {
                    user_id,
                    display_name: row.try_get("display_name")?,
                    system_admin: row.try_get::<i64, _>("system_admin")? != 0,
                    memberships,
                }))
            }
        }
    }

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

pub(super) fn hash_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn validate_token(token: &str) -> Result<(), StorageError> {
    if token.len() < 32 {
        return Err(StorageError::TokenTooShort);
    }
    Ok(())
}

fn as_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::LeaseDurationOverflow)
}
