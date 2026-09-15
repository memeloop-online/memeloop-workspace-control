use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, Row, SqliteConnection};
use utoipa::ToSchema;

use crate::{templates::WorkspacePlacement, workspaces::ResolvedPlacement};

use super::{Database, StorageError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct AvailableNodePool {
    pub name: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct NodePool {
    pub name: String,
    pub display_name: String,
    pub placement: ResolvedPlacement,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PutNodePool {
    pub display_name: String,
    #[serde(default)]
    pub placement: ResolvedPlacement,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

fn enabled_by_default() -> bool {
    true
}

impl Database {
    pub async fn list_available_node_pools(&self) -> Result<Vec<AvailableNodePool>, StorageError> {
        let rows = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                return sqlx::query("SELECT name, display_name FROM node_pools WHERE installation_id = ?1 AND enabled = 1 ORDER BY name")
                    .bind(installation_id.as_str())
                    .fetch_all(pool)
                    .await?
                    .into_iter()
                    .map(|row| Ok(AvailableNodePool { name: row.try_get("name")?, display_name: row.try_get("display_name")? }))
                    .collect();
            }
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query("SELECT name, display_name FROM node_pools WHERE installation_id = $1 AND enabled = 1 ORDER BY name")
                .bind(installation_id.as_str())
                .fetch_all(pool)
                .await?,
        };
        rows.into_iter()
            .map(|row| {
                Ok(AvailableNodePool {
                    name: row.try_get("name")?,
                    display_name: row.try_get("display_name")?,
                })
            })
            .collect()
    }

    pub async fn list_node_pools(&self) -> Result<Vec<NodePool>, StorageError> {
        let sql = "SELECT name, display_name, placement_json, enabled, created_at, updated_at FROM node_pools WHERE installation_id = {installation} ORDER BY name";
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(&sql.replace("{installation}", "?1"))
                .bind(installation_id.as_str())
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(decode_node_pool)
                .collect(),
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(&sql.replace("{installation}", "$1"))
                .bind(installation_id.as_str())
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(decode_node_pool)
                .collect(),
        }
    }

    pub async fn put_node_pool(
        &self,
        name: &str,
        command: &PutNodePool,
        now: i64,
    ) -> Result<NodePool, StorageError> {
        validate_node_pool(name, command)?;
        let placement_json = serde_json::to_string(&command.placement)?;
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => decode_node_pool(
                sqlx::query("INSERT INTO node_pools (installation_id, name, display_name, placement_json, enabled, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6) ON CONFLICT (installation_id, name) DO UPDATE SET display_name = excluded.display_name, placement_json = excluded.placement_json, enabled = excluded.enabled, updated_at = excluded.updated_at RETURNING name, display_name, placement_json, enabled, created_at, updated_at")
                    .bind(installation_id.as_str())
                    .bind(name)
                    .bind(&command.display_name)
                    .bind(&placement_json)
                    .bind(i64::from(command.enabled))
                    .bind(now)
                    .fetch_one(pool)
                    .await?,
            ),
            Self::Postgres {
                pool,
                installation_id,
            } => decode_node_pool(
                sqlx::query("INSERT INTO node_pools (installation_id, name, display_name, placement_json, enabled, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $6) ON CONFLICT (installation_id, name) DO UPDATE SET display_name = excluded.display_name, placement_json = excluded.placement_json, enabled = excluded.enabled, updated_at = excluded.updated_at RETURNING name, display_name, placement_json, enabled, created_at, updated_at")
                    .bind(installation_id.as_str())
                    .bind(name)
                    .bind(&command.display_name)
                    .bind(&placement_json)
                    .bind(i64::from(command.enabled))
                    .bind(now)
                    .fetch_one(pool)
                    .await?,
            ),
        }
    }

    pub async fn delete_node_pool(&self, name: &str) -> Result<(), StorageError> {
        if name == "default" {
            return Err(StorageError::NodePoolInUse);
        }
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                ensure_node_pool_deletable_sqlite(&mut transaction, installation_id.as_str(), name)
                    .await?;
                sqlx::query("DELETE FROM node_pools WHERE installation_id = ?1 AND name = ?2")
                    .bind(installation_id.as_str())
                    .bind(name)
                    .execute(&mut *transaction)
                    .await?;
                transaction.commit().await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                ensure_node_pool_deletable_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    name,
                )
                .await?;
                sqlx::query("DELETE FROM node_pools WHERE installation_id = $1 AND name = $2")
                    .bind(installation_id.as_str())
                    .bind(name)
                    .execute(&mut *transaction)
                    .await?;
                transaction.commit().await?;
            }
        }
        Ok(())
    }

    pub async fn resolve_workspace_placement(
        &self,
        node_pool: &str,
    ) -> Result<ResolvedPlacement, StorageError> {
        let placement_json: Option<String> = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query_scalar(
                "SELECT placement_json FROM node_pools WHERE installation_id = ?1 AND name = ?2",
            )
            .bind(installation_id.as_str())
            .bind(node_pool)
            .fetch_optional(pool)
            .await?,
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query_scalar(
                "SELECT placement_json FROM node_pools WHERE installation_id = $1 AND name = $2",
            )
            .bind(installation_id.as_str())
            .bind(node_pool)
            .fetch_optional(pool)
            .await?,
        };
        serde_json::from_str(
            placement_json
                .ok_or(StorageError::NodePoolNotFound)?
                .as_str(),
        )
        .map_err(StorageError::from)
    }
}

pub(super) async fn select_node_pool_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    placement: &WorkspacePlacement,
    requested: Option<&str>,
) -> Result<String, StorageError> {
    let name = selected_name(placement, requested)?;
    let enabled: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM node_pools WHERE installation_id = ?1 AND name = ?2 AND enabled = 1",
    )
    .bind(installation_id)
    .bind(name)
    .fetch_one(&mut *connection)
    .await?;
    (enabled == 1)
        .then(|| name.to_owned())
        .ok_or(StorageError::NodePoolUnavailable)
}

pub(super) async fn select_node_pool_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    placement: &WorkspacePlacement,
    requested: Option<&str>,
) -> Result<String, StorageError> {
    let name = selected_name(placement, requested)?;
    let enabled: Option<String> = sqlx::query_scalar("SELECT name FROM node_pools WHERE installation_id = $1 AND name = $2 AND enabled = 1 FOR SHARE")
        .bind(installation_id)
        .bind(name)
        .fetch_optional(&mut *connection)
        .await?;
    enabled.ok_or(StorageError::NodePoolUnavailable)
}

pub(super) async fn sync_template_placement_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    template_id: uuid::Uuid,
    placement: &WorkspacePlacement,
) -> Result<(), StorageError> {
    validate_template_pools_sqlite(connection, installation_id, placement).await?;
    sqlx::query(
        "DELETE FROM workspace_template_node_pools WHERE installation_id = ?1 AND template_id = ?2",
    )
    .bind(installation_id)
    .bind(template_id.to_string())
    .execute(&mut *connection)
    .await?;
    for pool in &placement.allowed_node_pools {
        sqlx::query("INSERT INTO workspace_template_node_pools (installation_id, template_id, node_pool, is_default) VALUES (?1, ?2, ?3, ?4)")
            .bind(installation_id)
            .bind(template_id.to_string())
            .bind(pool)
            .bind(i64::from(pool == &placement.default_node_pool))
            .execute(&mut *connection)
            .await?;
    }
    Ok(())
}

pub(super) async fn sync_template_placement_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    template_id: uuid::Uuid,
    placement: &WorkspacePlacement,
) -> Result<(), StorageError> {
    validate_template_pools_postgres(connection, installation_id, placement).await?;
    sqlx::query(
        "DELETE FROM workspace_template_node_pools WHERE installation_id = $1 AND template_id = $2",
    )
    .bind(installation_id)
    .bind(template_id.to_string())
    .execute(&mut *connection)
    .await?;
    for pool in &placement.allowed_node_pools {
        sqlx::query("INSERT INTO workspace_template_node_pools (installation_id, template_id, node_pool, is_default) VALUES ($1, $2, $3, $4)")
            .bind(installation_id)
            .bind(template_id.to_string())
            .bind(pool)
            .bind(i64::from(pool == &placement.default_node_pool))
            .execute(&mut *connection)
            .await?;
    }
    Ok(())
}

fn selected_name<'a>(
    placement: &'a WorkspacePlacement,
    requested: Option<&'a str>,
) -> Result<&'a str, StorageError> {
    let name = requested.unwrap_or(&placement.default_node_pool);
    placement
        .allowed_node_pools
        .iter()
        .any(|allowed| allowed == name)
        .then_some(name)
        .ok_or(StorageError::NodePoolNotAllowed)
}

async fn validate_template_pools_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    placement: &WorkspacePlacement,
) -> Result<(), StorageError> {
    for pool in &placement.allowed_node_pools {
        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM node_pools WHERE installation_id = ?1 AND name = ?2",
        )
        .bind(installation_id)
        .bind(pool)
        .fetch_one(&mut *connection)
        .await?;
        if exists != 1 {
            return Err(StorageError::NodePoolNotFound);
        }
    }
    Ok(())
}

async fn validate_template_pools_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    placement: &WorkspacePlacement,
) -> Result<(), StorageError> {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM node_pools WHERE installation_id = $1 AND name = ANY($2) FOR SHARE",
    )
    .bind(installation_id)
    .bind(&placement.allowed_node_pools)
    .fetch_all(&mut *connection)
    .await?;
    if rows.len() != placement.allowed_node_pools.len() {
        return Err(StorageError::NodePoolNotFound);
    }
    Ok(())
}

async fn ensure_node_pool_deletable_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    name: &str,
) -> Result<(), StorageError> {
    let row = sqlx::query("SELECT enabled, EXISTS(SELECT 1 FROM workspaces WHERE installation_id = ?1 AND node_pool = ?2 AND state <> 'deleted') workspace_in_use, EXISTS(SELECT 1 FROM workspace_template_node_pools WHERE installation_id = ?1 AND node_pool = ?2) template_in_use FROM node_pools WHERE installation_id = ?1 AND name = ?2")
        .bind(installation_id)
        .bind(name)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(StorageError::NodePoolNotFound)?;
    if row.try_get::<i64, _>("enabled")? != 0
        || row.try_get::<i64, _>("workspace_in_use")? != 0
        || row.try_get::<i64, _>("template_in_use")? != 0
    {
        return Err(StorageError::NodePoolInUse);
    }
    Ok(())
}

async fn ensure_node_pool_deletable_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    name: &str,
) -> Result<(), StorageError> {
    let row = sqlx::query("SELECT enabled, EXISTS(SELECT 1 FROM workspaces WHERE installation_id = $1 AND node_pool = $2 AND state <> 'deleted') workspace_in_use, EXISTS(SELECT 1 FROM workspace_template_node_pools WHERE installation_id = $1 AND node_pool = $2) template_in_use FROM node_pools WHERE installation_id = $1 AND name = $2 FOR UPDATE")
        .bind(installation_id)
        .bind(name)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(StorageError::NodePoolNotFound)?;
    if row.try_get::<i64, _>("enabled")? != 0
        || row.try_get::<bool, _>("workspace_in_use")?
        || row.try_get::<bool, _>("template_in_use")?
    {
        return Err(StorageError::NodePoolInUse);
    }
    Ok(())
}

fn decode_node_pool<R: Row>(row: R) -> Result<NodePool, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
    i64: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
{
    Ok(NodePool {
        name: row.try_get("name")?,
        display_name: row.try_get("display_name")?,
        placement: serde_json::from_str(&row.try_get::<String, _>("placement_json")?)?,
        enabled: row.try_get::<i64, _>("enabled")? != 0,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn validate_node_pool(name: &str, command: &PutNodePool) -> Result<(), StorageError> {
    if !valid_node_pool_name(name)
        || command.display_name.trim().is_empty()
        || command.display_name != command.display_name.trim()
        || command.display_name.len() > 80
        || command.display_name.chars().any(char::is_control)
        || command.placement.selector.len() > 32
        || command.placement.required_hosts.len() > 128
        || command.placement.preferred_hosts.len() > 128
        || command
            .placement
            .selector
            .iter()
            .any(|(key, value)| !valid_label_key(key) || !valid_label_value(value))
        || command
            .placement
            .required_hosts
            .iter()
            .chain(&command.placement.preferred_hosts)
            .any(|name| !valid_label_value(name) || name.is_empty())
    {
        return Err(StorageError::InvalidNodePool);
    }
    Ok(())
}

fn valid_node_pool_name(value: &str) -> bool {
    value.len() <= 63
        && !value.is_empty()
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value.bytes().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == b'-'
        })
}

fn valid_label_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
}

fn valid_label_value(value: &str) -> bool {
    value.len() <= 63
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}
