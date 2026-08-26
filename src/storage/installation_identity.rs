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
        Ok(())
    }
}
