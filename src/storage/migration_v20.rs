use sqlx::{PgConnection, Row, SqliteConnection};
use uuid::Uuid;

use crate::templates::WorkspaceTemplateDocument;

use super::StorageError;

pub(super) async fn upgrade_sqlite(
    connection: &mut SqliteConnection,
    applied_at: i64,
) -> Result<(), StorageError> {
    let templates = sqlx::query("SELECT id, template_yaml FROM workspace_templates")
        .fetch_all(&mut *connection)
        .await?;
    for row in templates {
        let yaml: String = row.try_get("template_yaml")?;
        let yaml = cleanse_yaml(&yaml, StorageError::InvalidTemplate)?;
        sqlx::query(
            "UPDATE workspace_templates SET template_yaml = ?1, updated_at = ?2 WHERE id = ?3",
        )
        .bind(yaml)
        .bind(applied_at)
        .bind(row.try_get::<String, _>("id")?)
        .execute(&mut *connection)
        .await?;
    }
    upgrade_workspaces_sqlite(connection, applied_at).await?;
    cleanse_idempotency_sqlite(connection).await
}

pub(super) async fn upgrade_postgres(
    connection: &mut PgConnection,
    applied_at: i64,
) -> Result<(), StorageError> {
    let templates = sqlx::query("SELECT id, template_yaml FROM workspace_templates FOR UPDATE")
        .fetch_all(&mut *connection)
        .await?;
    for row in templates {
        let yaml: String = row.try_get("template_yaml")?;
        let yaml = cleanse_yaml(&yaml, StorageError::InvalidTemplate)?;
        sqlx::query(
            "UPDATE workspace_templates SET template_yaml = $1, updated_at = $2 WHERE id = $3",
        )
        .bind(yaml)
        .bind(applied_at)
        .bind(row.try_get::<String, _>("id")?)
        .execute(&mut *connection)
        .await?;
    }
    upgrade_workspaces_postgres(connection, applied_at).await?;
    cleanse_idempotency_postgres(connection).await
}

async fn upgrade_workspaces_sqlite(
    connection: &mut SqliteConnection,
    applied_at: i64,
) -> Result<(), StorageError> {
    let rows = sqlx::query(
        "SELECT id, installation_id, state, generation, template_snapshot_yaml FROM workspaces",
    )
    .fetch_all(&mut *connection)
    .await?;
    for row in rows {
        let generation = next_generation(row.try_get("generation")?)?;
        let state: String = row.try_get("state")?;
        let id: String = row.try_get("id")?;
        let installation_id: String = row.try_get("installation_id")?;
        let yaml: String = row.try_get("template_snapshot_yaml")?;
        let yaml = cleanse_yaml(&yaml, StorageError::InvalidWorkspace)?;
        sqlx::query("UPDATE workspaces SET template_snapshot_yaml = ?1, generation = ?2, updated_at = ?3 WHERE id = ?4")
            .bind(yaml).bind(generation).bind(applied_at).bind(&id)
            .execute(&mut *connection).await?;
        if state != "deleted" && !has_active_lease_sqlite(connection, &id, applied_at).await? {
            insert_reconcile_sqlite(connection, &installation_id, &id, generation, applied_at)
                .await?;
        }
    }
    Ok(())
}

async fn upgrade_workspaces_postgres(
    connection: &mut PgConnection,
    applied_at: i64,
) -> Result<(), StorageError> {
    let rows = sqlx::query("SELECT id, installation_id, state, generation, template_snapshot_yaml FROM workspaces FOR UPDATE")
        .fetch_all(&mut *connection)
        .await?;
    for row in rows {
        let generation = next_generation(row.try_get("generation")?)?;
        let state: String = row.try_get("state")?;
        let id: String = row.try_get("id")?;
        let installation_id: String = row.try_get("installation_id")?;
        let yaml: String = row.try_get("template_snapshot_yaml")?;
        let yaml = cleanse_yaml(&yaml, StorageError::InvalidWorkspace)?;
        sqlx::query("UPDATE workspaces SET template_snapshot_yaml = $1, generation = $2, updated_at = $3 WHERE id = $4")
            .bind(yaml).bind(generation).bind(applied_at).bind(&id)
            .execute(&mut *connection).await?;
        if state != "deleted" && !has_active_lease_postgres(connection, &id, applied_at).await? {
            insert_reconcile_postgres(connection, &installation_id, &id, generation, applied_at)
                .await?;
        }
    }
    Ok(())
}

async fn has_active_lease_sqlite(
    connection: &mut SqliteConnection,
    workspace: &str,
    now: i64,
) -> Result<bool, StorageError> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM workspace_leases WHERE workspace_id = ?1 AND lease_expires_at > ?2",
    )
    .bind(workspace)
    .bind(now)
    .fetch_one(&mut *connection)
    .await?
        != 0)
}

async fn has_active_lease_postgres(
    connection: &mut PgConnection,
    workspace: &str,
    now: i64,
) -> Result<bool, StorageError> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT lease_expires_at FROM workspace_leases WHERE workspace_id = $1 AND lease_expires_at > $2 FOR SHARE",
    )
    .bind(workspace)
    .bind(now)
    .fetch_optional(&mut *connection)
    .await?
    .is_some())
}

async fn insert_reconcile_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    workspace: &str,
    generation: i64,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES (?1, ?2, 'reconcile_workspace', ?3, ?4, 'pending', ?5, NULL, NULL, 0, ?5, ?5)")
        .bind(Uuid::now_v7().to_string()).bind(installation).bind(workspace)
        .bind(serde_json::json!({"generation": generation, "reason": "schema_v20"}).to_string())
        .bind(now).execute(&mut *connection).await?;
    Ok(())
}

async fn insert_reconcile_postgres(
    connection: &mut PgConnection,
    installation: &str,
    workspace: &str,
    generation: i64,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES ($1, $2, 'reconcile_workspace', $3, $4, 'pending', $5, NULL, NULL, 0, $5, $5)")
        .bind(Uuid::now_v7().to_string()).bind(installation).bind(workspace)
        .bind(serde_json::json!({"generation": generation, "reason": "schema_v20"}).to_string())
        .bind(now).execute(&mut *connection).await?;
    Ok(())
}

async fn cleanse_idempotency_sqlite(connection: &mut SqliteConnection) -> Result<(), StorageError> {
    let rows = sqlx::query("SELECT installation_id, scope, key, response_json FROM idempotency_keys WHERE response_json <> ''")
        .fetch_all(&mut *connection).await?;
    for row in rows {
        update_response_sqlite(connection, row).await?;
    }
    Ok(())
}

async fn cleanse_idempotency_postgres(connection: &mut PgConnection) -> Result<(), StorageError> {
    let rows = sqlx::query("SELECT installation_id, scope, key, response_json FROM idempotency_keys WHERE response_json <> '' FOR UPDATE")
        .fetch_all(&mut *connection).await?;
    for row in rows {
        update_response_postgres(connection, row).await?;
    }
    Ok(())
}

async fn update_response_sqlite(
    connection: &mut SqliteConnection,
    row: sqlx::sqlite::SqliteRow,
) -> Result<(), StorageError> {
    let response = cleanse_response(&row.try_get::<String, _>("response_json")?)?;
    sqlx::query("UPDATE idempotency_keys SET response_json = ?1 WHERE installation_id = ?2 AND scope = ?3 AND key = ?4")
        .bind(response).bind(row.try_get::<String, _>("installation_id")?).bind(row.try_get::<String, _>("scope")?).bind(row.try_get::<String, _>("key")?)
        .execute(&mut *connection).await?;
    Ok(())
}

async fn update_response_postgres(
    connection: &mut PgConnection,
    row: sqlx::postgres::PgRow,
) -> Result<(), StorageError> {
    let response = cleanse_response(&row.try_get::<String, _>("response_json")?)?;
    sqlx::query("UPDATE idempotency_keys SET response_json = $1 WHERE installation_id = $2 AND scope = $3 AND key = $4")
        .bind(response).bind(row.try_get::<String, _>("installation_id")?).bind(row.try_get::<String, _>("scope")?).bind(row.try_get::<String, _>("key")?)
        .execute(&mut *connection).await?;
    Ok(())
}

fn cleanse_yaml(yaml: &str, invalid: StorageError) -> Result<String, StorageError> {
    let mut value: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(yaml).map_err(|_| StorageError::SchemaUpgradeDataInvalid)?;
    let spec = value
        .as_mapping_mut()
        .and_then(|root| root.get_mut(serde_yaml_ng::Value::String("spec".to_owned())))
        .and_then(serde_yaml_ng::Value::as_mapping_mut)
        .ok_or(StorageError::SchemaUpgradeDataInvalid)?;
    let environment = spec.remove(serde_yaml_ng::Value::String("environment".to_owned()));
    let environment_is_empty = match environment {
        None => true,
        Some(serde_yaml_ng::Value::Mapping(values)) => values.is_empty(),
        Some(_) => false,
    };
    if !environment_is_empty {
        return Err(StorageError::SchemaUpgradeDataInvalid);
    }
    spec.remove(serde_yaml_ng::Value::String(
        "preserve_home_ownership".to_owned(),
    ));
    spec.remove(serde_yaml_ng::Value::String(
        "preserve_home_root".to_owned(),
    ));
    let yaml =
        serde_yaml_ng::to_string(&value).map_err(|_| StorageError::SchemaUpgradeDataInvalid)?;
    WorkspaceTemplateDocument::parse(&yaml)
        .and_then(|document| document.to_yaml())
        .map_err(|_| invalid)
}

fn cleanse_response(value: &str) -> Result<String, StorageError> {
    let mut value: serde_json::Value =
        serde_json::from_str(value).map_err(|_| StorageError::SchemaUpgradeDataInvalid)?;
    let serde_json::Value::Object(response) = &mut value else {
        return Ok(value.to_string());
    };
    if let Some(workspace) = response.get_mut("workspace") {
        cleanse_response_spec(workspace)?;
    }
    if response.contains_key("yaml") && response.contains_key("enabled") {
        cleanse_response_spec(&mut value)?;
    }
    serde_json::to_string(&value).map_err(StorageError::from)
}

fn cleanse_response_spec(value: &mut serde_json::Value) -> Result<(), StorageError> {
    let serde_json::Value::Object(spec) = value else {
        return Err(StorageError::SchemaUpgradeDataInvalid);
    };
    if !(spec.contains_key("access_mode")
        && spec.contains_key("resources")
        && spec.contains_key("pod_requests"))
    {
        return Err(StorageError::SchemaUpgradeDataInvalid);
    }
    let environment_is_empty = match spec.get("environment") {
        None => true,
        Some(serde_json::Value::Object(values)) => values.is_empty(),
        Some(_) => false,
    };
    if !environment_is_empty {
        return Err(StorageError::SchemaUpgradeDataInvalid);
    }
    spec.remove("environment");
    spec.remove("preserve_home_ownership");
    spec.remove("preserve_home_root");
    Ok(())
}

fn next_generation(value: i64) -> Result<i64, StorageError> {
    value
        .checked_add(1)
        .ok_or(StorageError::SchemaUpgradeDataInvalid)
}

#[cfg(test)]
mod tests {
    use crate::{
        quota::Resources,
        storage::Database,
        templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
        workspaces::AccessMode,
    };

    use super::*;

    fn yaml_with_legacy_spec(extra: &str) -> String {
        WorkspaceTemplateDocument::new(
            "migration",
            WorkspaceTemplateSpec::standard(
                "registry.example/workspace:1",
                AccessMode::Internal,
                Resources {
                    cpu_millis: 1_000,
                    memory_mib: 1_024,
                    gpu_count: 0,
                    disk_gib: 20,
                },
            ),
        )
        .to_yaml()
        .unwrap()
        .replace("buildkit:", &format!("{extra}  buildkit:"))
    }

    #[test]
    fn cleans_empty_legacy_fields_before_final_template_validation() {
        let yaml = yaml_with_legacy_spec(
            "preserve_home_ownership: true\n  preserve_home_root: false\n  environment: {}\n",
        );
        let migrated = cleanse_yaml(&yaml, StorageError::InvalidTemplate).unwrap();
        assert!(!migrated.contains("preserve_home"));
        assert!(!migrated.contains("environment:"));
        assert!(WorkspaceTemplateDocument::parse(&migrated).is_ok());
    }

    #[test]
    fn rejects_nonempty_or_nonmapping_legacy_environment() {
        let nonempty = yaml_with_legacy_spec("environment: {TOKEN: secret}\n");
        let nonmapping = yaml_with_legacy_spec("environment: secret\n");
        assert!(matches!(
            cleanse_yaml(&nonempty, StorageError::InvalidTemplate),
            Err(StorageError::SchemaUpgradeDataInvalid)
        ));
        assert!(matches!(
            cleanse_yaml(&nonmapping, StorageError::InvalidTemplate),
            Err(StorageError::SchemaUpgradeDataInvalid)
        ));
    }

    #[test]
    fn response_cleanup_is_structural_and_never_deletes_unrelated_environment_values() {
        let unrelated = r#"{"environment":{"TOKEN":"secret"},"result":"ok"}"#;
        assert_eq!(cleanse_response(unrelated).unwrap(), unrelated);
        let workspace = r#"{"workspace":{"access_mode":"internal","resources":{},"pod_requests":{},"environment":{}}}"#;
        assert!(!cleanse_response(workspace).unwrap().contains("environment"));
        let legacy_secret = r#"{"workspace":{"access_mode":"internal","resources":{},"pod_requests":{},"environment":{"TOKEN":"secret"}}}"#;
        assert!(matches!(
            cleanse_response(legacy_secret),
            Err(StorageError::SchemaUpgradeDataInvalid)
        ));
    }

    async fn v19_sqlite_with_template(yaml: String) -> Database {
        let database = Database::connect("sqlite::memory:", "v20-bridge".parse().unwrap())
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let Database::Sqlite { pool, .. } = &database else {
            unreachable!();
        };
        sqlx::query("INSERT INTO workspace_templates (id, installation_id, organization_id, name, image, access_mode, cpu_millis, memory_mib, gpu_count, disk_gib, enabled, created_at, updated_at, template_yaml) VALUES (?1, ?2, NULL, 'migration', 'registry.example/workspace:1', 'internal', 1000, 1024, 0, 20, 1, 1, 1, ?3)")
            .bind(Uuid::now_v7().to_string())
            .bind("v20-bridge")
            .bind(yaml)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM schema_migrations")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES (19, 1)")
            .execute(pool)
            .await
            .unwrap();
        database
    }

    #[tokio::test]
    async fn sqlite_v19_bridge_commits_clean_templates_and_rolls_back_bad_environment() {
        let valid = yaml_with_legacy_spec("preserve_home_ownership: true\n  environment: {}\n");
        let database = v19_sqlite_with_template(valid).await;
        database.migrate().await.unwrap();
        assert_eq!(database.schema_version().await.unwrap(), 20);
        let Database::Sqlite { pool, .. } = &database else {
            unreachable!();
        };
        let yaml: String = sqlx::query_scalar("SELECT template_yaml FROM workspace_templates")
            .fetch_one(pool)
            .await
            .unwrap();
        assert!(!yaml.contains("preserve_home_ownership"));

        let invalid = yaml_with_legacy_spec("environment: {TOKEN: secret}\n");
        let failed = v19_sqlite_with_template(invalid).await;
        assert!(matches!(
            failed.migrate().await,
            Err(StorageError::SchemaUpgradeDataInvalid)
        ));
        assert_eq!(failed.schema_version().await.unwrap(), 19);
    }
}
