use std::str::FromStr;

use crate::config::{DatabaseMode, InstallationId};
use sqlx::{
    PgPool, SqlitePool,
    postgres::PgPoolOptions,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

mod admin_store;
mod audit_store;
mod error;
mod event_store;
mod idempotency;
mod identity;
mod image_policy_store;
mod injection_reconcile;
mod injection_store;
mod installation_identity;
mod job_store;
mod job_types;
mod leases;
mod metrics_store;
mod plugin_package_store;
mod plugin_store;
mod port_mappings;
mod schema;
mod ssh_access;
mod ssh_identity;
mod template_migration;
mod template_store;
mod transfer;
mod user_settings;
mod web_shell;
mod webhook_store;
mod workspace_actions;
mod workspace_admission;
mod workspace_events;
mod workspace_injection_refs;
mod workspace_store;

pub use admin_store::{
    AuditRecord, MembershipPage, MembershipSummary, OrganizationPage, UserPage, UserSummary,
};
pub use audit_store::{AuditFilter, AuditPage};
pub use error::StorageError;
pub use event_store::{EventNotifier, EventRecord};
pub use idempotency::{IdempotencyCompletion, IdempotencyDecision, IdempotencyReplay};
pub use identity::{CreateOrganization, InitialUserCommand, Membership, Organization, Principal};
pub use image_policy_store::ImagePolicy;
pub use injection_store::{InjectionScopeRef, StoredInjectionSummary};
pub use job_types::{ClaimedJob, NewJob};
pub use metrics_store::{JobCounts, UserWorkspaceMetrics, WorkspaceMetrics};
pub use plugin_package_store::{
    ConfirmPluginInstall, CreatePluginUiSession, PluginAssetBlob, PluginInstallInspection,
    PluginPackageRecord, PluginUiSession, StorePluginInspection,
};
pub use plugin_store::{PluginConfigurationWrite, StoredPluginConfiguration};
pub use port_mappings::{IssuedPortMappingTicket, PortMapping, hash_secret, validate_http_port};
pub use ssh_access::SshAccessCandidate;
pub use ssh_identity::{WorkspaceSshIdentity, WorkspaceSshPublicIdentity};
pub use template_store::{CreateWorkspaceTemplate, WorkspaceTemplate};
pub use transfer::DatabaseSnapshot;
pub(crate) use user_settings::validate_api_key_policy;
pub use user_settings::{ApiKeySummary, CreatedApiKey, StoredUserProfile};
pub use web_shell::{IssuedWebShellTicket, WebShellIdentity};
pub use webhook_store::{CreateWebhookSubscription, WebhookDelivery, WebhookSubscriptionSummary};
pub use workspace_injection_refs::WorkspaceInjectionRefs;
pub use workspace_store::{CreateWorkspace, WorkspacePage};

#[derive(Debug, Clone)]
pub enum Database {
    Sqlite {
        pool: SqlitePool,
        installation_id: InstallationId,
    },
    Postgres {
        pool: PgPool,
        installation_id: InstallationId,
    },
}

impl Database {
    pub async fn ping(&self) -> Result<(), StorageError> {
        match self {
            Self::Sqlite { pool, .. } => {
                sqlx::query("SELECT 1").execute(pool).await?;
            }
            Self::Postgres { pool, .. } => {
                sqlx::query("SELECT 1").execute(pool).await?;
            }
        }
        Ok(())
    }

    pub async fn connect(url: &str, installation_id: InstallationId) -> Result<Self, StorageError> {
        match DatabaseMode::from_url(url)? {
            DatabaseMode::Sqlite => {
                let options = SqliteConnectOptions::from_str(url)?
                    .create_if_missing(true)
                    .foreign_keys(true)
                    .journal_mode(SqliteJournalMode::Wal);
                let pool = SqlitePoolOptions::new()
                    .max_connections(1)
                    .connect_with(options)
                    .await?;
                Ok(Self::Sqlite {
                    pool,
                    installation_id,
                })
            }
            DatabaseMode::Postgres => {
                let pool = PgPoolOptions::new()
                    .max_connections(20)
                    .connect(url)
                    .await?;
                Ok(Self::Postgres {
                    pool,
                    installation_id,
                })
            }
        }
    }

    pub fn mode(&self) -> DatabaseMode {
        match self {
            Self::Sqlite { .. } => DatabaseMode::Sqlite,
            Self::Postgres { .. } => DatabaseMode::Postgres,
        }
    }

    pub fn installation_id(&self) -> &InstallationId {
        match self {
            Self::Sqlite {
                installation_id, ..
            }
            | Self::Postgres {
                installation_id, ..
            } => installation_id,
        }
    }

    pub async fn migrate(&self) -> Result<(), StorageError> {
        let applied_at = unix_timestamp()?;
        match self {
            Self::Sqlite { pool, .. } => migrate_sqlite(pool, applied_at).await?,
            Self::Postgres {
                pool,
                installation_id,
            } => migrate_postgres(pool, installation_id, applied_at).await?,
        }
        template_migration::backfill(self).await?;
        self.ensure_installation_identity().await
    }

    pub async fn schema_version(&self) -> Result<i64, StorageError> {
        let version = match self {
            Self::Sqlite { pool, .. } => {
                sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM schema_migrations")
                    .fetch_one(pool)
                    .await?
            }
            Self::Postgres { pool, .. } => {
                sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM schema_migrations")
                    .fetch_one(pool)
                    .await?
            }
        };
        Ok(version)
    }
}

async fn migrate_sqlite(pool: &SqlitePool, applied_at: i64) -> Result<(), StorageError> {
    let mut transaction = pool.begin().await?;
    sqlx::query(schema::MIGRATION_TABLE)
        .execute(&mut *transaction)
        .await?;
    let version =
        sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(version), 0) FROM schema_migrations")
            .fetch_one(&mut *transaction)
            .await?;
    if version < 8 {
        apply_sqlite_migration_group(&mut transaction, schema::MIGRATIONS).await?;
    }
    if version < 9 {
        apply_sqlite_migration_group(&mut transaction, schema::PROFILE_RENAME_MIGRATIONS).await?;
    }
    if version < 10 {
        apply_sqlite_migration_group(&mut transaction, schema::TEMPLATE_YAML_MIGRATIONS).await?;
    }
    if version < 11 {
        apply_sqlite_migration_group(&mut transaction, schema::PLUGIN_CONFIGURATION_MIGRATIONS)
            .await?;
    }
    if version < 13 {
        apply_sqlite_migration_group(&mut transaction, schema::DYNAMIC_PLUGIN_MIGRATIONS).await?;
    }
    if version < 14 {
        apply_sqlite_migration_group(&mut transaction, schema::USER_SETTINGS_MIGRATIONS).await?;
    }
    if version < 15 {
        apply_sqlite_migration_group(&mut transaction, user_settings::API_KEY_SCOPE_MIGRATIONS)
            .await?;
        apply_sqlite_migration_group(&mut transaction, schema::V15_MIGRATIONS).await?;
    }
    if version < 16 {
        apply_sqlite_migration_group(&mut transaction, schema::V16_MIGRATIONS).await?;
    }
    if version < schema::SCHEMA_VERSION {
        sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)")
            .bind(schema::SCHEMA_VERSION)
            .bind(applied_at)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    Ok(())
}

async fn apply_sqlite_migration_group(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    migrations: &[&str],
) -> Result<(), StorageError> {
    for migration in migrations {
        sqlx::query(migration).execute(&mut **transaction).await?;
    }
    Ok(())
}

async fn migrate_postgres(
    pool: &PgPool,
    installation_id: &InstallationId,
    applied_at: i64,
) -> Result<(), StorageError> {
    let mut transaction = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("mwc:migrate:{installation_id}"))
        .execute(&mut *transaction)
        .await?;
    sqlx::query(schema::MIGRATION_TABLE)
        .execute(&mut *transaction)
        .await?;
    let version =
        sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(version), 0) FROM schema_migrations")
            .fetch_one(&mut *transaction)
            .await?;
    if version < 8 {
        apply_postgres_migration_group(&mut transaction, schema::MIGRATIONS).await?;
    }
    if version < 9 {
        apply_postgres_migration_group(&mut transaction, schema::PROFILE_RENAME_MIGRATIONS).await?;
    }
    if version < 10 {
        apply_postgres_migration_group(&mut transaction, schema::TEMPLATE_YAML_MIGRATIONS).await?;
    }
    if version < 11 {
        apply_postgres_migration_group(&mut transaction, schema::PLUGIN_CONFIGURATION_MIGRATIONS)
            .await?;
    }
    if version < 13 {
        apply_postgres_migration_group(&mut transaction, schema::DYNAMIC_PLUGIN_MIGRATIONS).await?;
    }
    if version < 14 {
        apply_postgres_migration_group(&mut transaction, schema::USER_SETTINGS_MIGRATIONS).await?;
    }
    if version < 15 {
        apply_postgres_migration_group(&mut transaction, user_settings::API_KEY_SCOPE_MIGRATIONS)
            .await?;
        apply_postgres_migration_group(&mut transaction, schema::V15_MIGRATIONS).await?;
    }
    if version < 16 {
        apply_postgres_migration_group(&mut transaction, schema::V16_MIGRATIONS).await?;
    }
    if version < schema::SCHEMA_VERSION {
        sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES ($1, $2)")
            .bind(schema::SCHEMA_VERSION)
            .bind(applied_at)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    Ok(())
}

async fn apply_postgres_migration_group(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    migrations: &[&str],
) -> Result<(), StorageError> {
    for migration in migrations {
        sqlx::query(migration).execute(&mut **transaction).await?;
    }
    Ok(())
}

fn unix_timestamp() -> Result<i64, StorageError> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| StorageError::Clock)?
        .as_secs();
    i64::try_from(seconds).map_err(|_| StorageError::Clock)
}

#[cfg(test)]
mod schema_tests {
    use super::*;
    use sqlx::Row;
    use uuid::Uuid;

    #[tokio::test]
    async fn sqlite_membership_organization_role_index_has_expected_columns() {
        let database = Database::connect("sqlite::memory:", "schema-index-test".parse().unwrap())
            .await
            .unwrap();
        database.migrate().await.unwrap();

        let Database::Sqlite { pool, .. } = &database else {
            unreachable!("the test database is SQLite");
        };
        let rows = sqlx::query("PRAGMA index_info('memberships_organization_role_idx')")
            .fetch_all(pool)
            .await
            .unwrap();
        let columns = rows
            .into_iter()
            .map(|row| row.try_get::<String, _>("name").unwrap())
            .collect::<Vec<_>>();

        assert_eq!(columns, ["installation_id", "organization_id", "role"]);
        assert_eq!(database.schema_version().await.unwrap(), 16);

        // Re-running migrations must retain the same index rather than attempting to recreate it.
        database.migrate().await.unwrap();
        let index_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'memberships_organization_role_idx'",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(index_count, 1);
    }

    #[tokio::test]
    async fn postgres_membership_organization_role_index_has_expected_columns() {
        let Ok(database_url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
            eprintln!("skipping PostgreSQL schema index test: MWC_TEST_POSTGRES_URL is not set");
            return;
        };
        let schema = format!("mwc_schema_index_{}", Uuid::now_v7().simple());
        let administration = PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .unwrap();
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&administration)
            .await
            .unwrap();
        let mut scoped_url = url::Url::parse(&database_url).unwrap();
        scoped_url
            .query_pairs_mut()
            .append_pair("options", &format!("-c search_path={schema}"));
        let database = Database::connect(scoped_url.as_str(), "schema-index-test".parse().unwrap())
            .await
            .unwrap();
        database.migrate().await.unwrap();

        let Database::Postgres { pool, .. } = &database else {
            unreachable!("the test database is PostgreSQL");
        };
        let index_definition: String = sqlx::query_scalar(
            "SELECT indexdef FROM pg_indexes WHERE schemaname = current_schema() AND indexname = 'memberships_organization_role_idx'",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        assert!(index_definition.contains("(installation_id, organization_id, role)",));
        assert_eq!(database.schema_version().await.unwrap(), 16);

        drop(database);
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&administration)
            .await
            .unwrap();
        administration.close().await;
    }
}
