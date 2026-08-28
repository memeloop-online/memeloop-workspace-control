use std::str::FromStr;

use crate::config::{DatabaseMode, InstallationId};
use sqlx::{
    PgPool, SqlitePool,
    postgres::PgPoolOptions,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

mod admin_store;
mod catalog_store;
mod error;
mod event_store;
mod idempotency;
mod identity;
mod injection_reconcile;
mod injection_store;
mod installation_identity;
mod job_store;
mod job_types;
mod leases;
mod schema;
mod ssh_access;
mod ssh_identity;
mod transfer;
mod web_shell;
mod webhook_store;
mod workspace_actions;
mod workspace_admission;
mod workspace_events;
mod workspace_injection_refs;
mod workspace_store;

pub use admin_store::{
    AuditRecord, JobCounts, UserSummary, UserWorkspaceMetrics, WorkspaceMetrics,
};
pub use catalog_store::{CreateWorkspaceTemplate, ImagePolicy, WorkspaceTemplate};
pub use error::StorageError;
pub use event_store::{EventNotifier, EventRecord};
pub use idempotency::{IdempotencyDecision, IdempotencyReplay};
pub use identity::{CreateOrganization, Membership, Organization, Principal};
pub use injection_store::{InjectionScopeRef, StoredInjectionSummary};
pub use job_types::{ClaimedJob, NewJob};
pub use ssh_access::SshAccessCandidate;
pub use ssh_identity::{WorkspaceSshIdentity, WorkspaceSshPublicIdentity};
pub use transfer::DatabaseSnapshot;
pub use web_shell::{IssuedWebShellTicket, WebShellIdentity};
pub use webhook_store::{CreateWebhookSubscription, WebhookDelivery, WebhookSubscriptionSummary};
pub use workspace_injection_refs::WorkspaceInjectionRefs;
pub use workspace_store::CreateWorkspace;

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
            Self::Sqlite { pool, .. } => {
                let mut transaction = pool.begin().await?;
                sqlx::query(schema::MIGRATION_TABLE)
                    .execute(&mut *transaction)
                    .await?;
                let version = sqlx::query_scalar::<_, i64>(
                    "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                )
                .fetch_one(&mut *transaction)
                .await?;
                if version < 8 {
                    for migration in schema::MIGRATIONS {
                        sqlx::query(migration).execute(&mut *transaction).await?;
                    }
                }
                if version < 9 {
                    for migration in schema::PROFILE_RENAME_MIGRATIONS {
                        sqlx::query(migration).execute(&mut *transaction).await?;
                    }
                }
                if version < 10 {
                    for migration in schema::TEMPLATE_YAML_MIGRATIONS {
                        sqlx::query(migration).execute(&mut *transaction).await?;
                    }
                }
                if version < schema::SCHEMA_VERSION {
                    sqlx::query(
                        "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                    )
                    .bind(schema::SCHEMA_VERSION)
                    .bind(applied_at)
                    .execute(&mut *transaction)
                    .await?;
                }
                transaction.commit().await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut transaction = pool.begin().await?;
                sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                    .bind(format!("mwc:migrate:{installation_id}"))
                    .execute(&mut *transaction)
                    .await?;
                sqlx::query(schema::MIGRATION_TABLE)
                    .execute(&mut *transaction)
                    .await?;
                let version = sqlx::query_scalar::<_, i64>(
                    "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                )
                .fetch_one(&mut *transaction)
                .await?;
                if version < 8 {
                    for migration in schema::MIGRATIONS {
                        sqlx::query(migration).execute(&mut *transaction).await?;
                    }
                }
                if version < 9 {
                    for migration in schema::PROFILE_RENAME_MIGRATIONS {
                        sqlx::query(migration).execute(&mut *transaction).await?;
                    }
                }
                if version < 10 {
                    for migration in schema::TEMPLATE_YAML_MIGRATIONS {
                        sqlx::query(migration).execute(&mut *transaction).await?;
                    }
                }
                if version < schema::SCHEMA_VERSION {
                    sqlx::query(
                        "INSERT INTO schema_migrations (version, applied_at) VALUES ($1, $2)",
                    )
                    .bind(schema::SCHEMA_VERSION)
                    .bind(applied_at)
                    .execute(&mut *transaction)
                    .await?;
                }
                transaction.commit().await?;
            }
        }
        self.backfill_template_yaml().await?;
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

fn unix_timestamp() -> Result<i64, StorageError> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| StorageError::Clock)?
        .as_secs();
    i64::try_from(seconds).map_err(|_| StorageError::Clock)
}
