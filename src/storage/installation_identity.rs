use sqlx::Row;

use super::{Database, StorageError};

impl Database {
    pub(super) async fn ensure_installation_identity(&self) -> Result<(), StorageError> {
        let configured = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query(
                    "INSERT INTO installation_metadata (singleton, installation_id) \
                    VALUES (1, ?1) ON CONFLICT (singleton) DO NOTHING",
                )
                .bind(installation_id.as_str())
                .execute(pool)
                .await?;
                let row = sqlx::query(
                    "SELECT installation_id FROM installation_metadata WHERE singleton = 1",
                )
                .fetch_one(pool)
                .await?;
                (
                    installation_id,
                    row.try_get::<String, _>("installation_id")?,
                )
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query(
                    "INSERT INTO installation_metadata (singleton, installation_id) \
                    VALUES (1, $1) ON CONFLICT (singleton) DO NOTHING",
                )
                .bind(installation_id.as_str())
                .execute(pool)
                .await?;
                let row = sqlx::query(
                    "SELECT installation_id FROM installation_metadata WHERE singleton = 1",
                )
                .fetch_one(pool)
                .await?;
                (
                    installation_id,
                    row.try_get::<String, _>("installation_id")?,
                )
            }
        };
        if configured.0.as_str() != configured.1 {
            return Err(StorageError::InstallationMismatch {
                configured: configured.0.clone(),
                stored: configured.1,
            });
        }
        self.ensure_default_node_pool().await?;
        Ok(())
    }

    async fn ensure_default_node_pool(&self) -> Result<(), StorageError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| StorageError::Clock)?
            .as_secs();
        let now = i64::try_from(now).map_err(|_| StorageError::Clock)?;
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO node_pools (installation_id, name, display_name, placement_json, enabled, created_at, updated_at) VALUES (?1, 'default', 'Default', '{\"selector\":{},\"required_hosts\":[],\"preferred_hosts\":[]}', 1, ?2, ?2) ON CONFLICT (installation_id, name) DO NOTHING")
                    .bind(installation_id.as_str())
                    .bind(now)
                    .execute(pool)
                    .await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO node_pools (installation_id, name, display_name, placement_json, enabled, created_at, updated_at) VALUES ($1, 'default', 'Default', '{\"selector\":{},\"required_hosts\":[],\"preferred_hosts\":[]}', 1, $2, $2) ON CONFLICT (installation_id, name) DO NOTHING")
                    .bind(installation_id.as_str())
                    .bind(now)
                    .execute(pool)
                    .await?;
            }
        }
        Ok(())
    }
}
