use sqlx::Row;
use uuid::Uuid;

use crate::workspaces::Workspace;

use super::{
    Database, StorageError,
    workspace_store::{decode_postgres, decode_sqlite, select_workspace_by_short_id_sql},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshAccessCandidate {
    pub user_id: Uuid,
}

impl Database {
    pub async fn list_public_ssh_logins(&self) -> Result<Vec<String>, StorageError> {
        let sql = "SELECT short_id FROM workspaces WHERE installation_id = {install} AND access_mode = 'public' AND state = 'ready' ORDER BY short_id";
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(&sql.replace("{install}", "?1"))
                .bind(installation_id.as_str())
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(decode_login)
                .collect(),
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(&sql.replace("{install}", "$1"))
                .bind(installation_id.as_str())
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(decode_login)
                .collect(),
        }
    }

    pub async fn get_workspace_by_short_id(
        &self,
        short_id: &str,
    ) -> Result<Workspace, StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let row = sqlx::query(&select_workspace_by_short_id_sql("?1", "?2"))
                    .bind(installation_id.as_str())
                    .bind(short_id)
                    .fetch_optional(pool)
                    .await?;
                row.map(decode_sqlite)
                    .transpose()?
                    .ok_or(StorageError::WorkspaceNotFound)
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let row = sqlx::query(&select_workspace_by_short_id_sql("$1", "$2"))
                    .bind(installation_id.as_str())
                    .bind(short_id)
                    .fetch_optional(pool)
                    .await?;
                row.map(decode_postgres)
                    .transpose()?
                    .ok_or(StorageError::WorkspaceNotFound)
            }
        }
    }

    pub async fn ssh_access_candidates(
        &self,
        organization_id: Uuid,
    ) -> Result<Vec<SshAccessCandidate>, StorageError> {
        let rows = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                return sqlx::query("SELECT DISTINCT u.id FROM users u LEFT JOIN organization_memberships m ON m.installation_id = u.installation_id AND m.user_id = u.id AND m.organization_id = ?2 WHERE u.installation_id = ?1 AND u.disabled = 0 AND (u.system_admin <> 0 OR m.user_id IS NOT NULL) ORDER BY u.id")
                    .bind(installation_id.as_str()).bind(organization_id.to_string())
                    .fetch_all(pool).await?.into_iter().map(decode_candidate).collect();
            }
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query("SELECT DISTINCT u.id FROM users u LEFT JOIN organization_memberships m ON m.installation_id = u.installation_id AND m.user_id = u.id AND m.organization_id = $2 WHERE u.installation_id = $1 AND u.disabled = 0 AND (u.system_admin <> 0 OR m.user_id IS NOT NULL) ORDER BY u.id")
                .bind(installation_id.as_str()).bind(organization_id.to_string())
                .fetch_all(pool).await?,
        };
        rows.into_iter().map(decode_candidate).collect()
    }
}

fn decode_login<R: Row>(row: R) -> Result<String, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
{
    Ok(format!("access+{}", row.try_get::<String, _>("short_id")?))
}

fn decode_candidate<R: Row>(row: R) -> Result<SshAccessCandidate, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
{
    Ok(SshAccessCandidate {
        user_id: Uuid::parse_str(&row.try_get::<String, _>("id")?)?,
    })
}
