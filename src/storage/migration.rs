use std::collections::BTreeMap;

use crate::{config::InstallationId, templates::WorkspaceTemplateDocument};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row, SqlitePool};

use super::{Database, StorageError, schema};

pub(super) async fn migrate(database: &Database) -> Result<(), StorageError> {
    let applied_at = unix_timestamp()?;
    match database {
        Database::Sqlite {
            pool,
            installation_id,
        } => migrate_sqlite(pool, installation_id, applied_at).await?,
        Database::Postgres {
            pool,
            installation_id,
        } => migrate_postgres(pool, installation_id, applied_at).await?,
    }
    database.ensure_installation_identity().await
}

async fn migrate_sqlite(
    pool: &SqlitePool,
    installation_id: &InstallationId,
    applied_at: i64,
) -> Result<(), StorageError> {
    let mut transaction = pool.begin().await?;
    let has_application_tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' \
         AND name NOT IN ('schema_migrations', 'sqlite_sequence')",
    )
    .fetch_one(&mut *transaction)
    .await?;
    sqlx::query(schema::MIGRATION_TABLE)
        .execute(&mut *transaction)
        .await?;
    let version = current_sqlite_version(&mut transaction).await?;
    match version {
        schema::SCHEMA_VERSION => {}
        22 => {
            migrate_sqlite_22_to_23(&mut transaction, installation_id.as_str(), applied_at).await?;
            sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)")
                .bind(schema::SCHEMA_VERSION)
                .bind(applied_at)
                .execute(&mut *transaction)
                .await?;
        }
        0 if has_application_tables == 0 => {
            for statement in schema::BASELINE {
                sqlx::query(statement).execute(&mut *transaction).await?;
            }
            sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)")
                .bind(schema::SCHEMA_VERSION)
                .bind(applied_at)
                .execute(&mut *transaction)
                .await?;
        }
        _ => return Err(StorageError::UnsupportedDatabaseVersion),
    }
    transaction.commit().await?;
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
    let has_application_tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.tables \
         WHERE table_schema = current_schema() \
         AND table_name <> 'schema_migrations'",
    )
    .fetch_one(&mut *transaction)
    .await?;
    sqlx::query(schema::MIGRATION_TABLE)
        .execute(&mut *transaction)
        .await?;
    let version: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM schema_migrations")
            .fetch_one(&mut *transaction)
            .await?;
    match version {
        schema::SCHEMA_VERSION => {}
        22 => {
            migrate_postgres_22_to_23(&mut transaction, installation_id.as_str(), applied_at)
                .await?;
            sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES ($1, $2)")
                .bind(schema::SCHEMA_VERSION)
                .bind(applied_at)
                .execute(&mut *transaction)
                .await?;
        }
        0 if has_application_tables == 0 => {
            for statement in schema::BASELINE {
                sqlx::query(statement).execute(&mut *transaction).await?;
            }
            sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES ($1, $2)")
                .bind(schema::SCHEMA_VERSION)
                .bind(applied_at)
                .execute(&mut *transaction)
                .await?;
        }
        _ => return Err(StorageError::UnsupportedDatabaseVersion),
    }
    transaction.commit().await?;
    Ok(())
}

async fn migrate_sqlite_22_to_23(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    installation_id: &str,
    applied_at: i64,
) -> Result<(), StorageError> {
    for statement in [
        "CREATE TABLE node_pools (installation_id TEXT NOT NULL, name TEXT NOT NULL, display_name TEXT NOT NULL, placement_json TEXT NOT NULL, enabled BIGINT NOT NULL, created_at BIGINT NOT NULL, updated_at BIGINT NOT NULL, PRIMARY KEY (installation_id, name))",
        "ALTER TABLE organization_quotas ADD COLUMN temporary_storage_gib BIGINT NOT NULL DEFAULT 9223372036854775807",
        "ALTER TABLE user_quotas ADD COLUMN temporary_storage_gib BIGINT NOT NULL DEFAULT 9223372036854775807",
        "ALTER TABLE workspaces ADD COLUMN temporary_storage_gib BIGINT NOT NULL DEFAULT 22",
        "ALTER TABLE workspaces ADD COLUMN node_pool TEXT NOT NULL DEFAULT 'default'",
        "ALTER TABLE workspace_templates ADD COLUMN temporary_storage_gib BIGINT NOT NULL DEFAULT 22",
        "CREATE TABLE workspace_runtime_incidents (id TEXT PRIMARY KEY, installation_id TEXT NOT NULL, workspace_id TEXT NOT NULL, category TEXT NOT NULL CHECK (category IN ('disk_pressure', 'evicted', 'temporary_storage_provisioning', 'temporary_storage_attachment', 'volume_unavailable', 'other')), observed_at BIGINT NOT NULL, count BIGINT NOT NULL CHECK (count > 0), first_seen_at BIGINT NOT NULL, last_seen_at BIGINT NOT NULL CHECK (last_seen_at >= first_seen_at), UNIQUE (installation_id, workspace_id, category, observed_at), FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE)",
        "CREATE INDEX workspace_runtime_incidents_recent_idx ON workspace_runtime_incidents (installation_id, workspace_id, last_seen_at, observed_at)",
        "CREATE TABLE workspace_template_node_pools (installation_id TEXT NOT NULL, template_id TEXT NOT NULL, node_pool TEXT NOT NULL, is_default BIGINT NOT NULL, PRIMARY KEY (installation_id, template_id, node_pool), FOREIGN KEY (template_id) REFERENCES workspace_templates(id) ON DELETE CASCADE, FOREIGN KEY (installation_id, node_pool) REFERENCES node_pools(installation_id, name))",
        "CREATE UNIQUE INDEX workspace_template_default_pool_idx ON workspace_template_node_pools (installation_id, template_id) WHERE is_default = 1",
        "CREATE INDEX workspaces_node_pool_idx ON workspaces (installation_id, node_pool, state, id)",
    ] {
        sqlx::query(statement).execute(&mut **transaction).await?;
    }
    insert_imported_pool_sqlite(
        transaction,
        installation_id,
        "default",
        "Default",
        &crate::workspaces::ResolvedPlacement::default(),
        applied_at,
    )
    .await?;

    let template_rows =
        sqlx::query("SELECT id, template_yaml FROM workspace_templates WHERE installation_id = ?1")
            .bind(installation_id)
            .fetch_all(&mut **transaction)
            .await?;
    for row in template_rows {
        let id: String = row.try_get("id")?;
        let yaml: String = row.try_get("template_yaml")?;
        let migrated = migrate_template_document(&yaml)?;
        for pool in &migrated.pools {
            insert_imported_pool_sqlite(
                transaction,
                installation_id,
                &pool.name,
                &pool.display_name,
                &pool.placement,
                applied_at,
            )
            .await?;
        }
        sqlx::query("UPDATE workspace_templates SET template_yaml = ?1, temporary_storage_gib = ?2 WHERE installation_id = ?3 AND id = ?4")
            .bind(&migrated.yaml)
            .bind(as_i64(migrated.temporary_storage_gib)?)
            .bind(installation_id)
            .bind(&id)
            .execute(&mut **transaction)
            .await?;
        insert_template_placement_sqlite(transaction, installation_id, &id, &migrated.placement)
            .await?;
    }

    let workspace_rows =
        sqlx::query("SELECT id, template_snapshot_yaml FROM workspaces WHERE installation_id = ?1")
            .bind(installation_id)
            .fetch_all(&mut **transaction)
            .await?;
    for row in workspace_rows {
        let id: String = row.try_get("id")?;
        let yaml: String = row.try_get("template_snapshot_yaml")?;
        let migrated = migrate_template_document(&yaml)?;
        for pool in &migrated.pools {
            insert_imported_pool_sqlite(
                transaction,
                installation_id,
                &pool.name,
                &pool.display_name,
                &pool.placement,
                applied_at,
            )
            .await?;
        }
        sqlx::query("UPDATE workspaces SET template_snapshot_yaml = ?1, temporary_storage_gib = ?2, node_pool = ?3 WHERE installation_id = ?4 AND id = ?5")
            .bind(&migrated.yaml)
            .bind(as_i64(migrated.temporary_storage_gib)?)
            .bind(&migrated.placement.default_node_pool)
            .bind(installation_id)
            .bind(&id)
            .execute(&mut **transaction)
            .await?;
    }
    Ok(())
}

async fn migrate_postgres_22_to_23(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    installation_id: &str,
    applied_at: i64,
) -> Result<(), StorageError> {
    for statement in [
        "CREATE TABLE node_pools (installation_id TEXT NOT NULL, name TEXT NOT NULL, display_name TEXT NOT NULL, placement_json TEXT NOT NULL, enabled BIGINT NOT NULL, created_at BIGINT NOT NULL, updated_at BIGINT NOT NULL, PRIMARY KEY (installation_id, name))",
        "ALTER TABLE organization_quotas ADD COLUMN temporary_storage_gib BIGINT NOT NULL DEFAULT 9223372036854775807",
        "ALTER TABLE user_quotas ADD COLUMN temporary_storage_gib BIGINT NOT NULL DEFAULT 9223372036854775807",
        "ALTER TABLE workspaces ADD COLUMN temporary_storage_gib BIGINT NOT NULL DEFAULT 22",
        "ALTER TABLE workspaces ADD COLUMN node_pool TEXT NOT NULL DEFAULT 'default'",
        "ALTER TABLE workspace_templates ADD COLUMN temporary_storage_gib BIGINT NOT NULL DEFAULT 22",
        "CREATE TABLE workspace_runtime_incidents (id TEXT PRIMARY KEY, installation_id TEXT NOT NULL, workspace_id TEXT NOT NULL, category TEXT NOT NULL CHECK (category IN ('disk_pressure', 'evicted', 'temporary_storage_provisioning', 'temporary_storage_attachment', 'volume_unavailable', 'other')), observed_at BIGINT NOT NULL, count BIGINT NOT NULL CHECK (count > 0), first_seen_at BIGINT NOT NULL, last_seen_at BIGINT NOT NULL CHECK (last_seen_at >= first_seen_at), UNIQUE (installation_id, workspace_id, category, observed_at), FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE)",
        "CREATE INDEX workspace_runtime_incidents_recent_idx ON workspace_runtime_incidents (installation_id, workspace_id, last_seen_at, observed_at)",
        "CREATE TABLE workspace_template_node_pools (installation_id TEXT NOT NULL, template_id TEXT NOT NULL, node_pool TEXT NOT NULL, is_default BIGINT NOT NULL, PRIMARY KEY (installation_id, template_id, node_pool), FOREIGN KEY (template_id) REFERENCES workspace_templates(id) ON DELETE CASCADE, FOREIGN KEY (installation_id, node_pool) REFERENCES node_pools(installation_id, name))",
        "CREATE UNIQUE INDEX workspace_template_default_pool_idx ON workspace_template_node_pools (installation_id, template_id) WHERE is_default = 1",
        "CREATE INDEX workspaces_node_pool_idx ON workspaces (installation_id, node_pool, state, id)",
    ] {
        sqlx::query(statement).execute(&mut **transaction).await?;
    }
    insert_imported_pool_postgres(
        transaction,
        installation_id,
        "default",
        "Default",
        &crate::workspaces::ResolvedPlacement::default(),
        applied_at,
    )
    .await?;

    let template_rows =
        sqlx::query("SELECT id, template_yaml FROM workspace_templates WHERE installation_id = $1")
            .bind(installation_id)
            .fetch_all(&mut **transaction)
            .await?;
    for row in template_rows {
        let id: String = row.try_get("id")?;
        let yaml: String = row.try_get("template_yaml")?;
        let migrated = migrate_template_document(&yaml)?;
        for pool in &migrated.pools {
            insert_imported_pool_postgres(
                transaction,
                installation_id,
                &pool.name,
                &pool.display_name,
                &pool.placement,
                applied_at,
            )
            .await?;
        }
        sqlx::query("UPDATE workspace_templates SET template_yaml = $1, temporary_storage_gib = $2 WHERE installation_id = $3 AND id = $4")
            .bind(&migrated.yaml)
            .bind(as_i64(migrated.temporary_storage_gib)?)
            .bind(installation_id)
            .bind(&id)
            .execute(&mut **transaction)
            .await?;
        insert_template_placement_postgres(transaction, installation_id, &id, &migrated.placement)
            .await?;
    }

    let workspace_rows =
        sqlx::query("SELECT id, template_snapshot_yaml FROM workspaces WHERE installation_id = $1")
            .bind(installation_id)
            .fetch_all(&mut **transaction)
            .await?;
    for row in workspace_rows {
        let id: String = row.try_get("id")?;
        let yaml: String = row.try_get("template_snapshot_yaml")?;
        let migrated = migrate_template_document(&yaml)?;
        for pool in &migrated.pools {
            insert_imported_pool_postgres(
                transaction,
                installation_id,
                &pool.name,
                &pool.display_name,
                &pool.placement,
                applied_at,
            )
            .await?;
        }
        sqlx::query("UPDATE workspaces SET template_snapshot_yaml = $1, temporary_storage_gib = $2, node_pool = $3 WHERE installation_id = $4 AND id = $5")
            .bind(&migrated.yaml)
            .bind(as_i64(migrated.temporary_storage_gib)?)
            .bind(&migrated.placement.default_node_pool)
            .bind(installation_id)
            .bind(&id)
            .execute(&mut **transaction)
            .await?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct ImportedPool {
    name: String,
    display_name: String,
    placement: crate::workspaces::ResolvedPlacement,
}

struct MigratedTemplateDocument {
    yaml: String,
    temporary_storage_gib: u64,
    placement: crate::templates::WorkspacePlacement,
    pools: Vec<ImportedPool>,
}

fn migrate_template_document(yaml: &str) -> Result<MigratedTemplateDocument, StorageError> {
    let mut document: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(yaml).map_err(|_| StorageError::InvalidTemplate)?;
    let spec = mapping_value_mut(&mut document, "spec")?;
    let temporary_storage_gib = migrate_storage_policy(spec)?;
    spec.remove(&yaml_key("ephemeral_storage_limit_mib"));
    if let Some(requests) = spec
        .get_mut(&yaml_key("pod_requests"))
        .and_then(serde_yaml_ng::Value::as_mapping_mut)
    {
        requests.remove(&yaml_key("ephemeral_storage_mib"));
    }
    let previous_selector = string_map(spec.get(&yaml_key("node_selector")))?;
    let required_nodes = string_list(spec.get(&yaml_key("required_node_names")))?;
    let preferred_nodes = string_list(spec.get(&yaml_key("preferred_node_names")))?;
    spec.remove(&yaml_key("required_node_names"));
    spec.remove(&yaml_key("preferred_node_names"));
    spec.remove(&yaml_key("node_selector"));

    let (placement, pools) =
        migrated_placement(previous_selector, required_nodes, preferred_nodes)?;
    spec.insert(
        yaml_key("placement"),
        serde_yaml_ng::to_value(&placement).map_err(|_| StorageError::InvalidTemplate)?,
    );
    let yaml = serde_yaml_ng::to_string(&document).map_err(|_| StorageError::InvalidTemplate)?;
    WorkspaceTemplateDocument::parse(&yaml).map_err(|_| StorageError::InvalidTemplate)?;
    Ok(MigratedTemplateDocument {
        yaml,
        temporary_storage_gib,
        placement,
        pools,
    })
}

fn migrate_storage_policy(spec: &mut serde_yaml_ng::Mapping) -> Result<u64, StorageError> {
    let existing = spec
        .get(&yaml_key("storage_policy"))
        .and_then(serde_yaml_ng::Value::as_mapping);
    let explicit = existing
        .and_then(|policy| policy.get(&yaml_key("temporary_storage_gib")))
        .and_then(serde_yaml_ng::Value::as_u64);
    let previous_total = [
        ("build_scratch_gib", 12),
        ("buildkit_cache_gib", 8),
        ("codex_scratch_gib", 2),
    ]
    .into_iter()
    .map(|(key, default)| {
        existing
            .and_then(|policy| policy.get(&yaml_key(key)))
            .and_then(serde_yaml_ng::Value::as_u64)
            .unwrap_or(default)
    })
    .fold(0_u64, u64::saturating_add);
    let temporary_storage_gib = explicit.unwrap_or(previous_total).clamp(1, 2_048);
    spec.insert(
        yaml_key("storage_policy"),
        serde_yaml_ng::to_value(crate::templates::WorkspaceStoragePolicy {
            temporary_storage_gib,
        })
        .map_err(|_| StorageError::InvalidTemplate)?,
    );
    Ok(temporary_storage_gib)
}

fn migrated_placement(
    selector: BTreeMap<String, String>,
    mut required_nodes: Vec<String>,
    mut preferred_nodes: Vec<String>,
) -> Result<(crate::templates::WorkspacePlacement, Vec<ImportedPool>), StorageError> {
    if selector.is_empty() && required_nodes.is_empty() && preferred_nodes.is_empty() {
        return Ok((crate::templates::WorkspacePlacement::default(), Vec::new()));
    }
    required_nodes.sort_unstable();
    required_nodes.dedup();
    preferred_nodes.sort_unstable();
    preferred_nodes.dedup();
    let private_placement = crate::workspaces::ResolvedPlacement {
        selector,
        required_hosts: required_nodes,
        preferred_hosts: preferred_nodes,
    };
    let encoded = serde_json::to_vec(&private_placement)?;
    let digest = format!("{:x}", Sha256::digest(encoded));
    let name = format!("placement-{}", &digest[..24]);
    let placement = crate::templates::WorkspacePlacement {
        allowed_node_pools: vec![name.clone()],
        default_node_pool: name.clone(),
    };
    Ok((
        placement,
        vec![ImportedPool {
            name,
            display_name: "Custom placement".to_owned(),
            placement: private_placement,
        }],
    ))
}

fn mapping_value_mut<'a>(
    value: &'a mut serde_yaml_ng::Value,
    key: &str,
) -> Result<&'a mut serde_yaml_ng::Mapping, StorageError> {
    value
        .as_mapping_mut()
        .and_then(|mapping| mapping.get_mut(&yaml_key(key)))
        .and_then(serde_yaml_ng::Value::as_mapping_mut)
        .ok_or(StorageError::InvalidTemplate)
}

fn string_map(
    value: Option<&serde_yaml_ng::Value>,
) -> Result<BTreeMap<String, String>, StorageError> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    serde_yaml_ng::from_value(value.clone()).map_err(|_| StorageError::InvalidTemplate)
}

fn string_list(value: Option<&serde_yaml_ng::Value>) -> Result<Vec<String>, StorageError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    serde_yaml_ng::from_value(value.clone()).map_err(|_| StorageError::InvalidTemplate)
}

fn yaml_key(value: &str) -> serde_yaml_ng::Value {
    serde_yaml_ng::Value::String(value.to_owned())
}

async fn insert_imported_pool_sqlite(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    installation_id: &str,
    name: &str,
    display_name: &str,
    placement: &crate::workspaces::ResolvedPlacement,
    now: i64,
) -> Result<(), StorageError> {
    let placement_json = serde_json::to_string(placement)?;
    sqlx::query("INSERT INTO node_pools (installation_id, name, display_name, placement_json, enabled, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5) ON CONFLICT (installation_id, name) DO NOTHING")
        .bind(installation_id)
        .bind(name)
        .bind(display_name)
        .bind(&placement_json)
        .bind(now)
        .execute(&mut **transaction)
        .await?;
    let stored: String = sqlx::query_scalar(
        "SELECT placement_json FROM node_pools WHERE installation_id = ?1 AND name = ?2",
    )
    .bind(installation_id)
    .bind(name)
    .fetch_one(&mut **transaction)
    .await?;
    if stored != placement_json {
        return Err(StorageError::InvalidTemplate);
    }
    Ok(())
}

async fn insert_imported_pool_postgres(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    installation_id: &str,
    name: &str,
    display_name: &str,
    placement: &crate::workspaces::ResolvedPlacement,
    now: i64,
) -> Result<(), StorageError> {
    let placement_json = serde_json::to_string(placement)?;
    sqlx::query("INSERT INTO node_pools (installation_id, name, display_name, placement_json, enabled, created_at, updated_at) VALUES ($1, $2, $3, $4, 1, $5, $5) ON CONFLICT (installation_id, name) DO NOTHING")
        .bind(installation_id)
        .bind(name)
        .bind(display_name)
        .bind(&placement_json)
        .bind(now)
        .execute(&mut **transaction)
        .await?;
    let stored: String = sqlx::query_scalar(
        "SELECT placement_json FROM node_pools WHERE installation_id = $1 AND name = $2",
    )
    .bind(installation_id)
    .bind(name)
    .fetch_one(&mut **transaction)
    .await?;
    if stored != placement_json {
        return Err(StorageError::InvalidTemplate);
    }
    Ok(())
}

async fn insert_template_placement_sqlite(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    installation_id: &str,
    template_id: &str,
    placement: &crate::templates::WorkspacePlacement,
) -> Result<(), StorageError> {
    for pool in &placement.allowed_node_pools {
        sqlx::query("INSERT INTO workspace_template_node_pools (installation_id, template_id, node_pool, is_default) VALUES (?1, ?2, ?3, ?4)")
            .bind(installation_id)
            .bind(template_id)
            .bind(pool)
            .bind(i64::from(pool == &placement.default_node_pool))
            .execute(&mut **transaction)
            .await?;
    }
    Ok(())
}

async fn insert_template_placement_postgres(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    installation_id: &str,
    template_id: &str,
    placement: &crate::templates::WorkspacePlacement,
) -> Result<(), StorageError> {
    for pool in &placement.allowed_node_pools {
        sqlx::query("INSERT INTO workspace_template_node_pools (installation_id, template_id, node_pool, is_default) VALUES ($1, $2, $3, $4)")
            .bind(installation_id)
            .bind(template_id)
            .bind(pool)
            .bind(i64::from(pool == &placement.default_node_pool))
            .execute(&mut **transaction)
            .await?;
    }
    Ok(())
}

fn as_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::InvalidTemplate)
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

fn unix_timestamp() -> Result<i64, StorageError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).map_err(|_| StorageError::Clock))
        .map_err(|_| StorageError::Clock)?
}

#[cfg(test)]
mod current_tests {
    use super::*;
    use crate::config::InstallationId;

    #[tokio::test]
    async fn fresh_sqlite_bootstraps_current_schema_and_reaccepts_it() {
        let database = Database::connect("sqlite::memory:", "schema-test".parse().unwrap())
            .await
            .unwrap();
        database.migrate().await.unwrap();
        assert_eq!(
            database.schema_version().await.unwrap(),
            schema::SCHEMA_VERSION
        );
        database.migrate().await.unwrap();
    }

    #[tokio::test]
    async fn unsupported_sqlite_versions_are_rejected_without_conversion() {
        for version in [20, 21, 24] {
            let installation: InstallationId = "schema-test".parse().unwrap();
            let database = Database::connect("sqlite::memory:", installation)
                .await
                .unwrap();
            let Database::Sqlite { pool, .. } = &database else {
                unreachable!()
            };
            sqlx::query(schema::MIGRATION_TABLE)
                .execute(pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES (?1, 1)")
                .bind(version)
                .execute(pool)
                .await
                .unwrap();
            assert!(matches!(
                database.migrate().await,
                Err(StorageError::UnsupportedDatabaseVersion)
            ));
            assert_eq!(database.schema_version().await.unwrap(), version);
        }
    }

    #[tokio::test]
    async fn postgres_accepts_only_fresh_or_current_schema_when_configured() {
        let Ok(url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
            eprintln!("skipping PostgreSQL migration test: MWC_TEST_POSTGRES_URL is not set");
            return;
        };
        let admin = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        let fresh_schema = format!("mwc_schema_fresh_{}", uuid::Uuid::now_v7().simple());
        let fresh = postgres_database(&url, &fresh_schema, None).await;
        fresh.migrate().await.unwrap();
        assert_eq!(
            fresh.schema_version().await.unwrap(),
            schema::SCHEMA_VERSION
        );
        fresh.migrate().await.unwrap();
        drop(fresh);
        for version in [20, 21, 24] {
            let name = format!("mwc_schema_old_{}", uuid::Uuid::now_v7().simple());
            let database = postgres_database(&url, &name, Some(version)).await;
            assert!(matches!(
                database.migrate().await,
                Err(StorageError::UnsupportedDatabaseVersion)
            ));
            assert_eq!(database.schema_version().await.unwrap(), version);
            drop(database);
            sqlx::query(&format!("DROP SCHEMA {name} CASCADE"))
                .execute(&admin)
                .await
                .unwrap();
        }
        sqlx::query(&format!("DROP SCHEMA {fresh_schema} CASCADE"))
            .execute(&admin)
            .await
            .unwrap();
        admin.close().await;
    }

    async fn postgres_database(url: &str, schema_name: &str, version: Option<i64>) -> Database {
        let admin = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(url)
            .await
            .unwrap();
        sqlx::query(&format!("CREATE SCHEMA {schema_name}"))
            .execute(&admin)
            .await
            .unwrap();
        admin.close().await;
        let mut scoped = url::Url::parse(url).unwrap();
        scoped
            .query_pairs_mut()
            .append_pair("options", &format!("-c search_path={schema_name}"));
        let database = Database::connect(scoped.as_str(), "schema-pg".parse().unwrap())
            .await
            .unwrap();
        if let Some(version) = version {
            let Database::Postgres { pool, .. } = &database else {
                unreachable!()
            };
            sqlx::query(schema::MIGRATION_TABLE)
                .execute(pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES ($1, 1)")
                .bind(version)
                .execute(pool)
                .await
                .unwrap();
        }
        database
    }
}
