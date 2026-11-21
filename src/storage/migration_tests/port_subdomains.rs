use super::*;

async fn assert_backfilled(database: &Database) {
    database.migrate().await.unwrap();
    let mapping = database.get_port_mapping(uuid::Uuid::nil()).await.unwrap();
    assert_eq!(mapping.subdomain, "p-00000000000000000000000000000000");
    assert_eq!(mapping.internal_port, 3000);
    let claim_owner: String = match database {
        Database::Sqlite { pool, .. } => sqlx::query_scalar("SELECT workspace_id FROM workspace_port_subdomain_claims WHERE subdomain = 'p-00000000000000000000000000000000'").fetch_one(pool).await.unwrap(),
        Database::Postgres { pool, .. } => sqlx::query_scalar("SELECT workspace_id FROM workspace_port_subdomain_claims WHERE subdomain = 'p-00000000000000000000000000000000'").fetch_one(pool).await.unwrap(),
    };
    assert_eq!(claim_owner, mapping.workspace_id.to_string());
    assert_eq!(mapping.display_name.as_deref(), Some("Preserved mapping"));
    assert_eq!(
        database.schema_version().await.unwrap(),
        schema::SCHEMA_VERSION
    );
    database.migrate().await.unwrap();
    assert_eq!(
        database.get_port_mapping(mapping.id).await.unwrap(),
        mapping
    );
}

// The actual v31 table layout, without the new subdomain. Unrelated baseline
// tables stay identical; a real migration must preserve every existing row.
fn version_31_statements() -> Vec<String> {
    schema::BASELINE
        .iter()
        .filter(|statement| {
            !statement.starts_with("CREATE UNIQUE INDEX workspace_port_mappings_subdomain_idx")
                && !statement.starts_with("CREATE TABLE workspace_port_subdomain_claims")
        })
        .map(|statement| statement.replace(", subdomain TEXT NOT NULL", ""))
        .collect()
}

const PRESERVED_ROWS: &[&str] = &[
    "INSERT INTO users (id, installation_id, display_name, token_hash, system_admin, disabled, created_at) VALUES ('00000000-0000-0000-0000-000000000001', 'schema-test', 'Preserved', 'hash', 1, 0, 1)",
    "INSERT INTO organizations VALUES ('00000000-0000-0000-0000-000000000002', 'schema-test', 'Preserved org', 1)",
    "INSERT INTO workspaces (id, installation_id, short_id, organization_id, owner_id, name, image, access_mode, state, cpu_millis, memory_mib, gpu_count, disk_gib, temporary_storage_gib, node_pool, generation, created_at, updated_at, template_snapshot_yaml) VALUES ('00000000-0000-0000-0000-000000000003', 'schema-test', 'fixture', '00000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001', 'Preserved workspace', 'image', 'internal', 'stopped', 500, 512, 0, 5, 10, 'default', 1, 1, 1, 'unchanged')",
    "INSERT INTO workspace_port_mappings VALUES ('00000000-0000-0000-0000-000000000000', 'schema-test', '00000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000003', 3000, 'Preserved mapping', '00000000-0000-0000-0000-000000000001', 1)",
];

#[tokio::test]
async fn sqlite_31_backfills_existing_public_addresses_and_preserves_identity() {
    let database = Database::connect("sqlite::memory:", "schema-test".parse().unwrap())
        .await
        .unwrap();
    let Database::Sqlite { pool, .. } = &database else {
        unreachable!()
    };
    sqlx::query(schema::MIGRATION_TABLE)
        .execute(pool)
        .await
        .unwrap();
    for statement in version_31_statements() {
        sqlx::query(&statement).execute(pool).await.unwrap();
    }
    for statement in PRESERVED_ROWS {
        sqlx::query(statement).execute(pool).await.unwrap();
    }
    sqlx::query("INSERT INTO schema_migrations VALUES (31, 7)")
        .execute(pool)
        .await
        .unwrap();
    assert_backfilled(&database).await;
    let history: i64 =
        sqlx::query_scalar("SELECT applied_at FROM schema_migrations WHERE version = 31")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(history, 7);
}

#[tokio::test]
async fn postgres_31_backfills_existing_public_addresses_when_configured() {
    let Ok(url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        return;
    };
    let name = format!("mwc_port_subdomains_{}", uuid::Uuid::now_v7().simple());
    let mut database = postgres_database(&url, &name, Some(31)).await;
    let Database::Postgres {
        pool,
        installation_id,
    } = &mut database
    else {
        unreachable!()
    };
    *installation_id = "schema-test".parse().unwrap();
    for statement in version_31_statements() {
        sqlx::query(&statement).execute(&*pool).await.unwrap();
    }
    for statement in PRESERVED_ROWS {
        sqlx::query(statement).execute(&*pool).await.unwrap();
    }
    assert_backfilled(&database).await;
    let Database::Postgres { pool, .. } = &database else {
        unreachable!()
    };
    sqlx::query(&format!("DROP SCHEMA {name} CASCADE"))
        .execute(pool)
        .await
        .unwrap();
    pool.close().await;
}
