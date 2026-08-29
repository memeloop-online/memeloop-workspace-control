use serde::Serialize;
use serde_json::Value;
use sqlx::Row;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{Database, StorageError};

const MAX_CONFIGURATION_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct StoredPluginConfiguration {
    pub plugin_id: String,
    pub organization_id: Option<Uuid>,
    pub value: Value,
    pub schema_digest: String,
    pub version: u64,
    pub updated_at: i64,
}

pub struct PluginConfigurationWrite<'a> {
    pub plugin_id: &'a str,
    pub organization_id: Option<Uuid>,
    pub value: &'a Value,
    pub schema_digest: &'a str,
    pub expected_version: u64,
    pub actor_user_id: Uuid,
    pub now: i64,
}

impl Database {
    pub async fn plugin_configuration_for_scope(
        &self,
        plugin_id: &str,
        organization_id: Option<Uuid>,
    ) -> Result<Option<StoredPluginConfiguration>, StorageError> {
        validate_plugin_id(plugin_id)?;
        let scope_key = scope_key(organization_id);
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                return sqlx::query("SELECT plugin_id, organization_id, value_json, schema_digest, version, updated_at FROM plugin_configurations WHERE installation_id = ?1 AND plugin_id = ?2 AND scope_key = ?3")
                    .bind(installation_id.as_str())
                    .bind(plugin_id)
                    .bind(scope_key)
                    .fetch_optional(pool)
                    .await?
                    .map(decode_configuration)
                    .transpose();
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                return sqlx::query("SELECT plugin_id, organization_id, value_json, schema_digest, version, updated_at FROM plugin_configurations WHERE installation_id = $1 AND plugin_id = $2 AND scope_key = $3")
                    .bind(installation_id.as_str())
                    .bind(plugin_id)
                    .bind(scope_key)
                    .fetch_optional(pool)
                    .await?
                    .map(decode_configuration)
                    .transpose();
            }
        }
    }

    pub async fn put_plugin_configuration(
        &self,
        write: PluginConfigurationWrite<'_>,
    ) -> Result<StoredPluginConfiguration, StorageError> {
        let PluginConfigurationWrite {
            plugin_id,
            organization_id,
            value,
            schema_digest,
            expected_version,
            actor_user_id,
            now,
        } = write;
        validate_plugin_id(plugin_id)?;
        let expected = as_i64(expected_version)?;
        validate_digest(schema_digest)?;
        let encoded = serde_json::to_string(value)?;
        if encoded.len() > MAX_CONFIGURATION_BYTES {
            return Err(StorageError::InvalidPluginConfiguration);
        }
        let key = scope_key(organization_id);
        let kind = if organization_id.is_some() {
            "organization"
        } else {
            "installation"
        };
        let organization = organization_id.map(|id| id.to_string());
        let changed = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                require_organization_sqlite(
                    &mut transaction,
                    installation_id.as_str(),
                    organization_id,
                )
                .await?;
                let changed = if expected == 0 {
                    sqlx::query("INSERT INTO plugin_configurations (installation_id, plugin_id, scope_key, scope_kind, organization_id, value_json, schema_digest, version, updated_by, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,1,?8,?9) ON CONFLICT (installation_id, plugin_id, scope_key) DO NOTHING")
                        .bind(installation_id.as_str()).bind(plugin_id).bind(&key).bind(kind)
                        .bind(&organization).bind(&encoded).bind(schema_digest).bind(actor_user_id.to_string()).bind(now)
                        .execute(&mut *transaction).await?.rows_affected()
                } else {
                    sqlx::query("UPDATE plugin_configurations SET value_json = ?1, schema_digest = ?2, version = version + 1, updated_by = ?3, updated_at = ?4 WHERE installation_id = ?5 AND plugin_id = ?6 AND scope_key = ?7 AND version = ?8")
                        .bind(&encoded).bind(schema_digest).bind(actor_user_id.to_string()).bind(now)
                        .bind(installation_id.as_str()).bind(plugin_id).bind(&key).bind(expected)
                        .execute(&mut *transaction).await?.rows_affected()
                };
                if changed == 1 {
                    transaction.commit().await?;
                }
                changed
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                require_organization_postgres(
                    &mut transaction,
                    installation_id.as_str(),
                    organization_id,
                )
                .await?;
                let changed = if expected == 0 {
                    sqlx::query("INSERT INTO plugin_configurations (installation_id, plugin_id, scope_key, scope_kind, organization_id, value_json, schema_digest, version, updated_by, updated_at) VALUES ($1,$2,$3,$4,$5,$6,$7,1,$8,$9) ON CONFLICT (installation_id, plugin_id, scope_key) DO NOTHING")
                        .bind(installation_id.as_str()).bind(plugin_id).bind(&key).bind(kind)
                        .bind(&organization).bind(&encoded).bind(schema_digest).bind(actor_user_id.to_string()).bind(now)
                        .execute(&mut *transaction).await?.rows_affected()
                } else {
                    sqlx::query("UPDATE plugin_configurations SET value_json = $1, schema_digest = $2, version = version + 1, updated_by = $3, updated_at = $4 WHERE installation_id = $5 AND plugin_id = $6 AND scope_key = $7 AND version = $8")
                        .bind(&encoded).bind(schema_digest).bind(actor_user_id.to_string()).bind(now)
                        .bind(installation_id.as_str()).bind(plugin_id).bind(&key).bind(expected)
                        .execute(&mut *transaction).await?.rows_affected()
                };
                if changed == 1 {
                    transaction.commit().await?;
                }
                changed
            }
        };
        if changed != 1 {
            return Err(StorageError::PluginConfigurationVersionConflict);
        }
        Ok(StoredPluginConfiguration {
            plugin_id: plugin_id.to_owned(),
            organization_id,
            value: value.clone(),
            schema_digest: schema_digest.to_owned(),
            version: expected_version.saturating_add(1),
            updated_at: now,
        })
    }

    pub async fn delete_plugin_configuration(
        &self,
        plugin_id: &str,
        organization_id: Option<Uuid>,
        expected_version: u64,
    ) -> Result<(), StorageError> {
        validate_plugin_id(plugin_id)?;
        if expected_version == 0 {
            return Err(StorageError::PluginConfigurationVersionConflict);
        }
        let expected = as_i64(expected_version)?;
        let key = scope_key(organization_id);
        let changed = match self {
            Self::Sqlite { pool, installation_id } => sqlx::query("DELETE FROM plugin_configurations WHERE installation_id = ?1 AND plugin_id = ?2 AND scope_key = ?3 AND version = ?4")
                .bind(installation_id.as_str()).bind(plugin_id).bind(key).bind(expected).execute(pool).await?.rows_affected(),
            Self::Postgres { pool, installation_id } => sqlx::query("DELETE FROM plugin_configurations WHERE installation_id = $1 AND plugin_id = $2 AND scope_key = $3 AND version = $4")
                .bind(installation_id.as_str()).bind(plugin_id).bind(key).bind(expected).execute(pool).await?.rows_affected(),
        };
        if changed != 1 {
            return Err(StorageError::PluginConfigurationVersionConflict);
        }
        Ok(())
    }
}

fn decode_configuration<R: Row>(row: R) -> Result<StoredPluginConfiguration, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
    i64: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
{
    let organization: Option<String> = row.try_get("organization_id")?;
    Ok(StoredPluginConfiguration {
        plugin_id: row.try_get("plugin_id")?,
        organization_id: organization.map(|id| Uuid::parse_str(&id)).transpose()?,
        value: serde_json::from_str(&row.try_get::<String, _>("value_json")?)?,
        schema_digest: row.try_get("schema_digest")?,
        version: u64::try_from(row.try_get::<i64, _>("version")?)
            .map_err(|_| StorageError::InvalidPluginConfiguration)?,
        updated_at: row.try_get("updated_at")?,
    })
}

async fn require_organization_sqlite(
    connection: &mut sqlx::SqliteConnection,
    installation_id: &str,
    organization_id: Option<Uuid>,
) -> Result<(), StorageError> {
    let Some(organization_id) = organization_id else {
        return Ok(());
    };
    let exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM organizations WHERE installation_id = ?1 AND id = ?2",
    )
    .bind(installation_id)
    .bind(organization_id.to_string())
    .fetch_one(connection)
    .await?;
    if exists != 1 {
        return Err(StorageError::OrganizationNotFound);
    }
    Ok(())
}

async fn require_organization_postgres(
    connection: &mut sqlx::PgConnection,
    installation_id: &str,
    organization_id: Option<Uuid>,
) -> Result<(), StorageError> {
    let Some(organization_id) = organization_id else {
        return Ok(());
    };
    let exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM organizations WHERE installation_id = $1 AND id = $2",
    )
    .bind(installation_id)
    .bind(organization_id.to_string())
    .fetch_one(connection)
    .await?;
    if exists != 1 {
        return Err(StorageError::OrganizationNotFound);
    }
    Ok(())
}

fn scope_key(organization_id: Option<Uuid>) -> String {
    organization_id.map_or_else(
        || "installation".to_owned(),
        |id| format!("organization:{id}"),
    )
}

fn validate_plugin_id(value: &str) -> Result<(), StorageError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(StorageError::InvalidPluginConfiguration);
    }
    Ok(())
}

fn validate_digest(value: &str) -> Result<(), StorageError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(StorageError::InvalidPluginConfiguration);
    }
    Ok(())
}

fn as_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::InvalidPluginConfiguration)
}

#[cfg(test)]
mod tests {
    use crate::{config::InstallationId, storage::CreateOrganization};

    use super::*;

    #[tokio::test]
    async fn configuration_versions_are_optimistic_and_scoped() {
        let database = Database::connect(
            "sqlite::memory:",
            "plugins".parse::<InstallationId>().unwrap(),
        )
        .await
        .unwrap();
        database.migrate().await.unwrap();
        let admin = database
            .create_user("Admin", "plugin-test-token-000000000000000000000", true, 1)
            .await
            .unwrap();
        let organization = database
            .create_organization(
                CreateOrganization {
                    name: "Org".into(),
                    owner_user_id: admin.user_id,
                },
                2,
            )
            .await
            .unwrap();
        let digest = "a".repeat(64);
        let installation = database
            .put_plugin_configuration(PluginConfigurationWrite {
                plugin_id: "policy",
                organization_id: None,
                value: &serde_json::json!({"limit":1}),
                schema_digest: &digest,
                expected_version: 0,
                actor_user_id: admin.user_id,
                now: 3,
            })
            .await
            .unwrap();
        assert_eq!(installation.version, 1);
        let org = database
            .put_plugin_configuration(PluginConfigurationWrite {
                plugin_id: "policy",
                organization_id: Some(organization.id),
                value: &serde_json::json!({"limit":2}),
                schema_digest: &digest,
                expected_version: 0,
                actor_user_id: admin.user_id,
                now: 4,
            })
            .await
            .unwrap();
        assert_eq!(org.organization_id, Some(organization.id));
        assert!(matches!(
            database
                .put_plugin_configuration(PluginConfigurationWrite {
                    plugin_id: "policy",
                    organization_id: Some(organization.id),
                    value: &serde_json::json!({"limit":3}),
                    schema_digest: &digest,
                    expected_version: 0,
                    actor_user_id: admin.user_id,
                    now: 5,
                })
                .await,
            Err(StorageError::PluginConfigurationVersionConflict)
        ));
        database
            .delete_plugin_configuration("policy", Some(organization.id), 1)
            .await
            .unwrap();
        assert!(
            database
                .plugin_configuration_for_scope("policy", Some(organization.id))
                .await
                .unwrap()
                .is_none()
        );
    }
}
