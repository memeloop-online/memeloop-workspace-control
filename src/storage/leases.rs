use std::time::Duration;

use uuid::Uuid;

use super::{Database, StorageError};

impl Database {
    pub async fn try_acquire_workspace_lease(
        &self,
        workspace_id: Uuid,
        lease_owner: &str,
        now: i64,
        duration: Duration,
    ) -> Result<bool, StorageError> {
        if lease_owner.is_empty() {
            return Err(StorageError::EmptyLeaseOwner);
        }
        let expires_at = now
            .checked_add(
                i64::try_from(duration.as_secs())
                    .map_err(|_| StorageError::LeaseDurationOverflow)?,
            )
            .ok_or(StorageError::LeaseDurationOverflow)?;
        let affected = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(
                "INSERT INTO workspace_leases (installation_id, workspace_id, lease_owner, \
                lease_expires_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5) \
                ON CONFLICT (installation_id, workspace_id) DO UPDATE SET \
                lease_owner = excluded.lease_owner, lease_expires_at = excluded.lease_expires_at, \
                updated_at = excluded.updated_at WHERE workspace_leases.lease_expires_at < ?5 \
                OR workspace_leases.lease_owner = ?3",
            )
            .bind(installation_id.as_str())
            .bind(workspace_id.to_string())
            .bind(lease_owner)
            .bind(expires_at)
            .bind(now)
            .execute(pool)
            .await?
            .rows_affected(),
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(
                "INSERT INTO workspace_leases (installation_id, workspace_id, lease_owner, \
                lease_expires_at, updated_at) VALUES ($1, $2, $3, $4, $5) \
                ON CONFLICT (installation_id, workspace_id) DO UPDATE SET \
                lease_owner = excluded.lease_owner, lease_expires_at = excluded.lease_expires_at, \
                updated_at = excluded.updated_at WHERE workspace_leases.lease_expires_at < $5 \
                OR workspace_leases.lease_owner = $3",
            )
            .bind(installation_id.as_str())
            .bind(workspace_id.to_string())
            .bind(lease_owner)
            .bind(expires_at)
            .bind(now)
            .execute(pool)
            .await?
            .rows_affected(),
        };
        Ok(affected == 1)
    }

    pub async fn release_workspace_lease(
        &self,
        workspace_id: Uuid,
        lease_owner: &str,
    ) -> Result<(), StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query("DELETE FROM workspace_leases WHERE installation_id = ?1 AND workspace_id = ?2 AND lease_owner = ?3")
                    .bind(installation_id.as_str()).bind(workspace_id.to_string()).bind(lease_owner)
                    .execute(pool).await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query("DELETE FROM workspace_leases WHERE installation_id = $1 AND workspace_id = $2 AND lease_owner = $3")
                    .bind(installation_id.as_str()).bind(workspace_id.to_string()).bind(lease_owner)
                    .execute(pool).await?;
            }
        }
        Ok(())
    }
}
