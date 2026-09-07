use crate::config::InstallationId;
use sqlx::{PgPool, SqlitePool};

use super::{Database, StorageError, schema, template_migration, user_settings};

type MigrationGroup = (i64, &'static [&'static str]);

const COMMON_MIGRATION_GROUPS: &[MigrationGroup] = &[
    (8, schema::MIGRATIONS),
    (9, schema::PROFILE_RENAME_MIGRATIONS),
    (10, schema::TEMPLATE_YAML_MIGRATIONS),
    (11, schema::PLUGIN_CONFIGURATION_MIGRATIONS),
    (13, schema::DYNAMIC_PLUGIN_MIGRATIONS),
    (14, schema::USER_SETTINGS_MIGRATIONS),
    (15, user_settings::API_KEY_SCOPE_MIGRATIONS),
    (15, schema::V15_MIGRATIONS),
    (16, schema::V16_MIGRATIONS),
];

pub(super) async fn migrate(database: &Database) -> Result<(), StorageError> {
    let applied_at = unix_timestamp()?;
    match database {
        Database::Sqlite { pool, .. } => migrate_sqlite(pool, applied_at).await?,
        Database::Postgres {
            pool,
            installation_id,
        } => migrate_postgres(pool, installation_id, applied_at).await?,
    }
    template_migration::backfill(database).await?;
    database.ensure_installation_identity().await
}

async fn migrate_sqlite(pool: &SqlitePool, applied_at: i64) -> Result<(), StorageError> {
    let mut transaction = pool.begin().await?;
    sqlx::query(schema::MIGRATION_TABLE)
        .execute(&mut *transaction)
        .await?;
    let version = current_sqlite_version(&mut transaction).await?;
    for (_, statements) in COMMON_MIGRATION_GROUPS
        .iter()
        .chain(
            [
                (17, schema::V17_SQLITE_MIGRATIONS),
                (18, schema::V18_SQLITE_MIGRATIONS),
            ]
            .iter(),
        )
        .filter(|(introduced_in, _)| version < *introduced_in)
    {
        for statement in *statements {
            sqlx::query(statement).execute(&mut *transaction).await?;
        }
    }
    if version < 19 {
        ensure_sqlite_v19_runtime_is_canonical(&mut transaction).await?;
        for statement in schema::V19_SQLITE_MIGRATIONS {
            sqlx::query(statement).execute(&mut *transaction).await?;
        }
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

async fn current_sqlite_version(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<i64, StorageError> {
    Ok(
        sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM schema_migrations")
            .fetch_one(&mut **transaction)
            .await?,
    )
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
    let version: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM schema_migrations")
            .fetch_one(&mut *transaction)
            .await?;
    for (_, statements) in COMMON_MIGRATION_GROUPS
        .iter()
        .chain(
            [
                (17, schema::V17_POSTGRES_MIGRATIONS),
                (18, schema::V18_POSTGRES_MIGRATIONS),
            ]
            .iter(),
        )
        .filter(|(introduced_in, _)| version < *introduced_in)
    {
        for statement in *statements {
            sqlx::query(statement).execute(&mut *transaction).await?;
        }
    }
    if version < 19 {
        ensure_postgres_v19_runtime_is_canonical(&mut transaction).await?;
        for statement in schema::V19_POSTGRES_MIGRATIONS {
            sqlx::query(statement).execute(&mut *transaction).await?;
        }
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

/// The dropped v18 fields must be recoverable from the remaining immutable
/// placement data. Dedicated namespaces have a deterministic name; shared
/// namespaces are intentionally retained as validated persisted placement.
async fn ensure_sqlite_v19_runtime_is_canonical(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<(), StorageError> {
    let invalid_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM workspaces WHERE \
         runtime_naming_scheme <> 'prefixed_v2' \
         OR runtime_namespace_scope NOT IN ('dedicated', 'shared') \
         OR runtime_resource_prefix <> ('w-' || short_id) \
         OR runtime_route_key <> (installation_id || '-' || short_id) \
         OR short_id <> substr(replace(lower(id), '-', ''), 17, 16) \
         OR (runtime_namespace_scope = 'dedicated' \
             AND runtime_namespace <> ('ws-' || installation_id || '-' || short_id)) \
         OR (runtime_namespace_scope = 'shared' AND (\
             runtime_namespace = '' \
             OR length(runtime_namespace) > 63 \
             OR runtime_namespace GLOB '*[^a-z0-9-]*' \
             OR substr(runtime_namespace, 1, 1) NOT GLOB '[a-z0-9]' \
             OR substr(runtime_namespace, -1, 1) NOT GLOB '[a-z0-9]'\
         ))",
    )
    .fetch_one(&mut **transaction)
    .await?;
    if invalid_count != 0 {
        return Err(StorageError::WorkspaceRuntimeSchemaIncompatible);
    }
    Ok(())
}

async fn ensure_postgres_v19_runtime_is_canonical(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), StorageError> {
    let invalid_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM workspaces WHERE \
         runtime_naming_scheme <> 'prefixed_v2' \
         OR runtime_namespace_scope NOT IN ('dedicated', 'shared') \
         OR runtime_resource_prefix <> ('w-' || short_id) \
         OR runtime_route_key <> (installation_id || '-' || short_id) \
         OR short_id <> substr(replace(lower(id), '-', ''), 17, 16) \
         OR (runtime_namespace_scope = 'dedicated' \
             AND runtime_namespace <> ('ws-' || installation_id || '-' || short_id)) \
         OR (runtime_namespace_scope = 'shared' \
             AND runtime_namespace !~ '^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$')",
    )
    .fetch_one(&mut **transaction)
    .await?;
    if invalid_count != 0 {
        return Err(StorageError::WorkspaceRuntimeSchemaIncompatible);
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
mod tests {
    use sqlx::{Row, postgres::PgPoolOptions};
    use uuid::Uuid;

    use super::*;

    const WILDCARD_TOKEN: &str = "wildcard-migration-token-0000000000000000000000";

    async fn restore_sqlite_v18_runtime_schema(pool: &SqlitePool) {
        for statement in [
            "ALTER TABLE workspaces ADD COLUMN runtime_naming_scheme TEXT NOT NULL DEFAULT 'legacy_v1'",
            "ALTER TABLE workspaces ADD COLUMN runtime_resource_prefix TEXT NOT NULL DEFAULT 'workspace'",
            "ALTER TABLE workspaces ADD COLUMN runtime_route_key TEXT NOT NULL DEFAULT ''",
            "CREATE UNIQUE INDEX workspaces_runtime_route_key_idx ON workspaces (installation_id, runtime_route_key)",
            "CREATE UNIQUE INDEX workspaces_runtime_resource_idx ON workspaces (installation_id, runtime_namespace, runtime_resource_prefix)",
        ] {
            sqlx::query(statement).execute(pool).await.unwrap();
        }
        sqlx::query("UPDATE schema_migrations SET version = 18 WHERE version = 19")
            .execute(pool)
            .await
            .unwrap();
    }

    async fn restore_postgres_v18_runtime_schema(pool: &PgPool) {
        for statement in [
            "ALTER TABLE workspaces ADD COLUMN runtime_naming_scheme TEXT NOT NULL DEFAULT 'legacy_v1'",
            "ALTER TABLE workspaces ADD COLUMN runtime_resource_prefix TEXT NOT NULL DEFAULT 'workspace'",
            "ALTER TABLE workspaces ADD COLUMN runtime_route_key TEXT NOT NULL DEFAULT ''",
            "CREATE OR REPLACE FUNCTION workspaces_legacy_runtime_defaults() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END; $$",
            "CREATE TRIGGER workspaces_legacy_runtime_defaults_before_insert BEFORE INSERT ON workspaces FOR EACH ROW EXECUTE FUNCTION workspaces_legacy_runtime_defaults()",
            "CREATE UNIQUE INDEX workspaces_runtime_route_key_idx ON workspaces (installation_id, runtime_route_key)",
            "CREATE UNIQUE INDEX workspaces_runtime_resource_idx ON workspaces (installation_id, runtime_namespace, runtime_resource_prefix)",
        ] {
            sqlx::query(statement).execute(pool).await.unwrap();
        }
        sqlx::query("UPDATE schema_migrations SET version = 18 WHERE version = 19")
            .execute(pool)
            .await
            .unwrap();
    }

    async fn downgrade_sqlite_to_v16(pool: &SqlitePool) {
        for column in ["runtime_namespace_scope", "runtime_namespace"] {
            sqlx::query(&format!("ALTER TABLE workspaces DROP COLUMN {column}"))
                .execute(pool)
                .await
                .unwrap();
        }
        sqlx::query("UPDATE schema_migrations SET version = 16 WHERE version = 19")
            .execute(pool)
            .await
            .unwrap();
    }

    async fn downgrade_postgres_to_v16(pool: &PgPool) {
        for column in ["runtime_namespace_scope", "runtime_namespace"] {
            sqlx::query(&format!("ALTER TABLE workspaces DROP COLUMN {column}"))
                .execute(pool)
                .await
                .unwrap();
        }
        sqlx::query("UPDATE schema_migrations SET version = 16 WHERE version = 19")
            .execute(pool)
            .await
            .unwrap();
    }

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
        assert_eq!(database.schema_version().await.unwrap(), 19);

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
    async fn sqlite_empty_database_initializes_the_v19_runtime_schema() {
        let database = Database::connect("sqlite::memory:", "schema-v19-empty".parse().unwrap())
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let Database::Sqlite { pool, .. } = &database else {
            unreachable!("the test database is SQLite");
        };
        let columns = sqlx::query("PRAGMA table_info('workspaces')")
            .fetch_all(pool)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.try_get::<String, _>("name").unwrap())
            .collect::<Vec<_>>();
        assert!(columns.contains(&"runtime_namespace_scope".to_owned()));
        assert!(columns.contains(&"runtime_namespace".to_owned()));
        for removed in [
            "runtime_naming_scheme",
            "runtime_resource_prefix",
            "runtime_route_key",
        ] {
            assert!(!columns.contains(&removed.to_owned()));
        }
        assert_eq!(database.schema_version().await.unwrap(), 19);
    }

    #[tokio::test]
    async fn sqlite_v18_to_v19_drops_derived_runtime_identity_and_is_idempotent() {
        let database = Database::connect("sqlite::memory:", "legacy-install".parse().unwrap())
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let Database::Sqlite { pool, .. } = &database else {
            unreachable!("the test database is SQLite");
        };
        restore_sqlite_v18_runtime_schema(pool).await;
        let user_id = Uuid::now_v7().to_string();
        let organization_id = Uuid::now_v7().to_string();
        let workspace_id = Uuid::now_v7();
        let short_id = workspace_id.simple().to_string()[16..].to_owned();
        sqlx::query("INSERT INTO users (id, installation_id, display_name, token_hash, system_admin, disabled, created_at) VALUES (?1, 'legacy-install', 'Legacy user', 'legacy-hash', 0, 0, 1)")
            .bind(&user_id)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO organizations (id, installation_id, name, created_at) VALUES (?1, 'legacy-install', 'Legacy org', 1)")
            .bind(&organization_id)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO workspaces (id, installation_id, short_id, organization_id, owner_id, name, image, access_mode, state, cpu_millis, memory_mib, gpu_count, disk_gib, generation, created_at, updated_at, runtime_naming_scheme, runtime_namespace_scope, runtime_namespace, runtime_resource_prefix, runtime_route_key) VALUES (?1, 'legacy-install', ?2, ?3, ?4, 'Canonical workspace', 'registry.example/canonical:1', 'internal', 'ready', 1000, 2048, 0, 20, 1, 1, 1, 'prefixed_v2', 'shared', 'shared-runtime', ?5, ?6)")
            .bind(workspace_id.to_string())
            .bind(&short_id)
            .bind(&organization_id)
            .bind(&user_id)
            .bind(format!("w-{short_id}"))
            .bind(format!("legacy-install-{short_id}"))
            .execute(pool)
            .await
            .unwrap();

        database.migrate().await.unwrap();

        let workspace_columns = sqlx::query("PRAGMA table_info('workspaces')")
            .fetch_all(pool)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.try_get::<String, _>("name").unwrap())
            .collect::<Vec<_>>();
        assert!(workspace_columns.contains(&"runtime_namespace_scope".to_owned()));
        assert!(workspace_columns.contains(&"runtime_namespace".to_owned()));
        let placement: (String, String) = sqlx::query_as(
            "SELECT runtime_namespace_scope, runtime_namespace FROM workspaces WHERE id = ?1",
        )
        .bind(workspace_id.to_string())
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(
            placement,
            ("shared".to_owned(), "shared-runtime".to_owned())
        );
        for removed in [
            "runtime_naming_scheme",
            "runtime_resource_prefix",
            "runtime_route_key",
        ] {
            assert!(!workspace_columns.contains(&removed.to_owned()));
        }
        let removed_indexes: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name IN ('workspaces_runtime_route_key_idx', 'workspaces_runtime_resource_idx')",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(removed_indexes, 0);
        assert_eq!(database.schema_version().await.unwrap(), 19);

        database.migrate().await.unwrap();
        assert_eq!(database.schema_version().await.unwrap(), 19);
    }

    #[tokio::test]
    async fn sqlite_v19_rejects_noncanonical_v18_runtime_and_rolls_back() {
        let database = Database::connect("sqlite::memory:", "legacy-install".parse().unwrap())
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let Database::Sqlite { pool, .. } = &database else {
            unreachable!("the test database is SQLite");
        };
        restore_sqlite_v18_runtime_schema(pool).await;
        let user_id = Uuid::now_v7().to_string();
        let organization_id = Uuid::now_v7().to_string();
        let workspace_id = Uuid::now_v7();
        let short_id = workspace_id.simple().to_string()[16..].to_owned();
        sqlx::query("INSERT INTO users (id, installation_id, display_name, token_hash, system_admin, disabled, created_at) VALUES (?1, 'legacy-install', 'Legacy user', 'legacy-hash', 0, 0, 1)")
            .bind(&user_id)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO organizations (id, installation_id, name, created_at) VALUES (?1, 'legacy-install', 'Legacy org', 1)")
            .bind(&organization_id)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO workspaces (id, installation_id, short_id, organization_id, owner_id, name, image, access_mode, state, cpu_millis, memory_mib, gpu_count, disk_gib, generation, created_at, updated_at, runtime_naming_scheme, runtime_namespace_scope, runtime_namespace, runtime_resource_prefix, runtime_route_key) VALUES (?1, 'legacy-install', ?2, ?3, ?4, 'Legacy workspace', 'registry.example/legacy:1', 'internal', 'ready', 1000, 2048, 0, 20, 1, 1, 1, 'legacy_v1', 'dedicated', ?5, 'workspace', ?2)")
            .bind(workspace_id.to_string())
            .bind(&short_id)
            .bind(&organization_id)
            .bind(&user_id)
            .bind(format!("ws-legacy-install-{short_id}"))
            .execute(pool)
            .await
            .unwrap();

        assert!(matches!(
            database.migrate().await,
            Err(StorageError::WorkspaceRuntimeSchemaIncompatible)
        ));
        let retained_columns = sqlx::query("PRAGMA table_info('workspaces')")
            .fetch_all(pool)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.try_get::<String, _>("name").unwrap())
            .collect::<Vec<_>>();
        for retained in [
            "runtime_naming_scheme",
            "runtime_namespace_scope",
            "runtime_namespace",
            "runtime_resource_prefix",
            "runtime_route_key",
        ] {
            assert!(retained_columns.contains(&retained.to_owned()));
        }
        let retained_indexes: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name IN ('workspaces_runtime_route_key_idx', 'workspaces_runtime_resource_idx')",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(retained_indexes, 2);
        assert_eq!(database.schema_version().await.unwrap(), 18);
    }

    #[tokio::test]
    async fn sqlite_v17_removes_wildcard_and_unbounded_api_keys() {
        let database = Database::connect("sqlite::memory:", "schema-key-cleanup".parse().unwrap())
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let user = database
            .create_user("Key owner", WILDCARD_TOKEN, true, 1)
            .await
            .unwrap();
        let now = unix_timestamp().unwrap();
        let unbounded = database
            .create_api_key(
                user.user_id,
                "Temporary key",
                vec![crate::auth::ApiKeyScope::ReadWorkspace],
                Some(now + 60),
                now,
            )
            .await
            .unwrap();

        let Database::Sqlite { pool, .. } = &database else {
            unreachable!("the test database is SQLite");
        };
        sqlx::query("UPDATE user_api_keys SET scopes_json = '[\"*\"]' WHERE token_hash = ?1")
            .bind(crate::storage::identity::hash_token(WILDCARD_TOKEN))
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("UPDATE user_api_keys SET expires_at = NULL WHERE id = ?1")
            .bind(unbounded.summary.id.to_string())
            .execute(pool)
            .await
            .unwrap();
        downgrade_sqlite_to_v16(pool).await;

        database.migrate().await.unwrap();
        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_api_keys")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(remaining, 0);
        assert!(
            database
                .authenticate(WILDCARD_TOKEN)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            database
                .authenticate(&unbounded.token)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn postgres_v17_removes_wildcard_and_unbounded_api_keys() {
        let Ok(database_url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
            eprintln!(
                "skipping PostgreSQL API-key migration test: MWC_TEST_POSTGRES_URL is not set"
            );
            return;
        };
        let schema = format!("mwc_key_cleanup_{}", Uuid::now_v7().simple());
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
        let database =
            Database::connect(scoped_url.as_str(), "schema-key-cleanup".parse().unwrap())
                .await
                .unwrap();
        database.migrate().await.unwrap();
        let user = database
            .create_user("Key owner", WILDCARD_TOKEN, true, 1)
            .await
            .unwrap();
        let now = unix_timestamp().unwrap();
        let unbounded = database
            .create_api_key(
                user.user_id,
                "Temporary key",
                vec![crate::auth::ApiKeyScope::ReadWorkspace],
                Some(now + 60),
                now,
            )
            .await
            .unwrap();

        let Database::Postgres { pool, .. } = &database else {
            unreachable!("the test database is PostgreSQL");
        };
        sqlx::query("UPDATE user_api_keys SET scopes_json = '[\"*\"]' WHERE token_hash = $1")
            .bind(crate::storage::identity::hash_token(WILDCARD_TOKEN))
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("UPDATE user_api_keys SET expires_at = NULL WHERE id = $1")
            .bind(unbounded.summary.id.to_string())
            .execute(pool)
            .await
            .unwrap();
        downgrade_postgres_to_v16(pool).await;

        database.migrate().await.unwrap();
        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_api_keys")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(remaining, 0);
        assert!(
            database
                .authenticate(WILDCARD_TOKEN)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            database
                .authenticate(&unbounded.token)
                .await
                .unwrap()
                .is_none()
        );

        drop(database);
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&administration)
            .await
            .unwrap();
        administration.close().await;
    }

    #[tokio::test]
    async fn postgres_v18_to_v19_drops_derived_fields_and_compatibility_trigger() {
        let Ok(database_url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
            eprintln!(
                "skipping PostgreSQL runtime migration test: MWC_TEST_POSTGRES_URL is not set"
            );
            return;
        };
        let schema = format!("mwc_runtime_migration_{}", Uuid::now_v7().simple());
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
        let database = Database::connect(scoped_url.as_str(), "legacy-install".parse().unwrap())
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let Database::Postgres { pool, .. } = &database else {
            unreachable!("the test database is PostgreSQL");
        };
        restore_postgres_v18_runtime_schema(pool).await;
        let user_id = Uuid::now_v7().to_string();
        let organization_id = Uuid::now_v7().to_string();
        let workspace_id = Uuid::now_v7();
        let short_id = workspace_id.simple().to_string()[16..].to_owned();
        sqlx::query("INSERT INTO users (id, installation_id, display_name, token_hash, system_admin, disabled, created_at) VALUES ($1, 'legacy-install', 'Legacy user', 'legacy-hash', 0, 0, 1)")
            .bind(&user_id)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO organizations (id, installation_id, name, created_at) VALUES ($1, 'legacy-install', 'Legacy org', 1)")
            .bind(&organization_id)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO workspaces (id, installation_id, short_id, organization_id, owner_id, name, image, access_mode, state, cpu_millis, memory_mib, gpu_count, disk_gib, generation, created_at, updated_at, runtime_naming_scheme, runtime_namespace_scope, runtime_namespace, runtime_resource_prefix, runtime_route_key) VALUES ($1, 'legacy-install', $2, $3, $4, 'Canonical workspace', 'registry.example/canonical:1', 'internal', 'ready', 1000, 2048, 0, 20, 1, 1, 1, 'prefixed_v2', 'dedicated', $5, $6, $7)")
            .bind(workspace_id.to_string())
            .bind(&short_id)
            .bind(&organization_id)
            .bind(&user_id)
            .bind(format!("ws-legacy-install-{short_id}"))
            .bind(format!("w-{short_id}"))
            .bind(format!("legacy-install-{short_id}"))
            .execute(pool)
            .await
            .unwrap();

        database.migrate().await.unwrap();

        let retained_columns: Vec<String> = sqlx::query_scalar(
            "SELECT column_name FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = 'workspaces'",
        )
        .fetch_all(pool)
        .await
        .unwrap();
        assert!(retained_columns.contains(&"runtime_namespace_scope".to_owned()));
        assert!(retained_columns.contains(&"runtime_namespace".to_owned()));
        for removed in [
            "runtime_naming_scheme",
            "runtime_resource_prefix",
            "runtime_route_key",
        ] {
            assert!(!retained_columns.contains(&removed.to_owned()));
        }
        let remaining_compatibility_objects: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pg_trigger WHERE tgrelid = 'workspaces'::regclass AND tgname = 'workspaces_legacy_runtime_defaults_before_insert'",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(remaining_compatibility_objects, 0);
        let remaining_function: bool = sqlx::query_scalar(
            "SELECT to_regprocedure('workspaces_legacy_runtime_defaults()') IS NOT NULL",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        assert!(!remaining_function);
        assert_eq!(database.schema_version().await.unwrap(), 19);

        database.migrate().await.unwrap();
        assert_eq!(database.schema_version().await.unwrap(), 19);

        drop(database);
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&administration)
            .await
            .unwrap();
        administration.close().await;
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
        assert_eq!(database.schema_version().await.unwrap(), 19);

        drop(database);
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&administration)
            .await
            .unwrap();
        administration.close().await;
    }
}
