use sqlx::{PgConnection, Row, SqliteConnection};

use crate::{
    quota::{ResourceQuota, Resources},
    workspaces::Workspace,
};

use super::StorageError;

pub(super) async fn admit_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace: &Workspace,
) -> Result<(), StorageError> {
    super::catalog_store::admit_sqlite(connection, installation_id, workspace).await?;
    let quota = sqlx::query("SELECT cpu_millis, memory_mib, gpu_count, disk_gib FROM organization_quotas WHERE installation_id = ?1 AND organization_id = ?2")
        .bind(installation_id).bind(workspace.organization_id.to_string())
        .fetch_optional(&mut *connection).await?;
    if let Some(quota) = quota {
        let usage = sqlx::query("SELECT COALESCE(SUM(cpu_millis), 0) cpu_millis, COALESCE(SUM(memory_mib), 0) memory_mib, COALESCE(SUM(gpu_count), 0) gpu_count, COALESCE(SUM(disk_gib), 0) disk_gib FROM workspaces WHERE installation_id = ?1 AND organization_id = ?2 AND state <> 'deleted'")
            .bind(installation_id).bind(workspace.organization_id.to_string())
            .fetch_one(&mut *connection).await?;
        admit_rows(&quota, &usage, workspace.template.resources)?;
    }
    let quota = sqlx::query("SELECT cpu_millis, memory_mib, gpu_count, disk_gib FROM user_quotas WHERE installation_id = ?1 AND user_id = ?2")
        .bind(installation_id).bind(workspace.owner_id.to_string())
        .fetch_optional(&mut *connection).await?;
    let Some(quota) = quota else { return Ok(()) };
    let usage = sqlx::query("SELECT COALESCE(SUM(cpu_millis), 0) cpu_millis, COALESCE(SUM(memory_mib), 0) memory_mib, COALESCE(SUM(gpu_count), 0) gpu_count, COALESCE(SUM(disk_gib), 0) disk_gib FROM workspaces WHERE installation_id = ?1 AND owner_id = ?2 AND state <> 'deleted'")
        .bind(installation_id).bind(workspace.owner_id.to_string())
        .fetch_one(&mut *connection).await?;
    admit_rows(&quota, &usage, workspace.template.resources)
}

pub(super) async fn admit_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &Workspace,
) -> Result<(), StorageError> {
    super::catalog_store::admit_postgres(connection, installation_id, workspace).await?;
    let quota = sqlx::query("SELECT cpu_millis, memory_mib, gpu_count, disk_gib FROM organization_quotas WHERE installation_id = $1 AND organization_id = $2 FOR UPDATE")
        .bind(installation_id).bind(workspace.organization_id.to_string())
        .fetch_optional(&mut *connection).await?;
    if let Some(quota) = quota {
        let usage = sqlx::query("SELECT CAST(COALESCE(SUM(cpu_millis), 0) AS BIGINT) cpu_millis, CAST(COALESCE(SUM(memory_mib), 0) AS BIGINT) memory_mib, CAST(COALESCE(SUM(gpu_count), 0) AS BIGINT) gpu_count, CAST(COALESCE(SUM(disk_gib), 0) AS BIGINT) disk_gib FROM workspaces WHERE installation_id = $1 AND organization_id = $2 AND state <> 'deleted'")
            .bind(installation_id).bind(workspace.organization_id.to_string())
            .fetch_one(&mut *connection).await?;
        admit_rows(&quota, &usage, workspace.template.resources)?;
    }
    let quota = sqlx::query("SELECT cpu_millis, memory_mib, gpu_count, disk_gib FROM user_quotas WHERE installation_id = $1 AND user_id = $2 FOR UPDATE")
        .bind(installation_id).bind(workspace.owner_id.to_string())
        .fetch_optional(&mut *connection).await?;
    let Some(quota) = quota else { return Ok(()) };
    let usage = sqlx::query("SELECT CAST(COALESCE(SUM(cpu_millis), 0) AS BIGINT) cpu_millis, CAST(COALESCE(SUM(memory_mib), 0) AS BIGINT) memory_mib, CAST(COALESCE(SUM(gpu_count), 0) AS BIGINT) gpu_count, CAST(COALESCE(SUM(disk_gib), 0) AS BIGINT) disk_gib FROM workspaces WHERE installation_id = $1 AND owner_id = $2 AND state <> 'deleted'")
        .bind(installation_id).bind(workspace.owner_id.to_string())
        .fetch_one(&mut *connection).await?;
    admit_rows(&quota, &usage, workspace.template.resources)
}

fn admit_rows<Q: Row, U: Row>(quota: &Q, usage: &U, request: Resources) -> Result<(), StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<Q> + sqlx::ColumnIndex<U>,
    i64: for<'decode> sqlx::Decode<'decode, Q::Database>
        + sqlx::Type<Q::Database>
        + for<'decode> sqlx::Decode<'decode, U::Database>
        + sqlx::Type<U::Database>,
{
    let decode = |row: &Q, key| row.try_get::<i64, _>(key);
    let limit = Resources {
        cpu_millis: as_u64(decode(quota, "cpu_millis")?)?,
        memory_mib: as_u64(decode(quota, "memory_mib")?)?,
        gpu_count: as_u32(decode(quota, "gpu_count")?)?,
        disk_gib: as_u64(decode(quota, "disk_gib")?)?,
    };
    let current = Resources {
        cpu_millis: as_u64(usage.try_get::<i64, _>("cpu_millis")?)?,
        memory_mib: as_u64(usage.try_get::<i64, _>("memory_mib")?)?,
        gpu_count: as_u32(usage.try_get::<i64, _>("gpu_count")?)?,
        disk_gib: as_u64(usage.try_get::<i64, _>("disk_gib")?)?,
    };
    ResourceQuota { limit }.admit(current, request)?;
    Ok(())
}

fn as_u64(value: i64) -> Result<u64, StorageError> {
    u64::try_from(value).map_err(|_| StorageError::InvalidWorkspace)
}

fn as_u32(value: i64) -> Result<u32, StorageError> {
    u32::try_from(value).map_err(|_| StorageError::InvalidWorkspace)
}
