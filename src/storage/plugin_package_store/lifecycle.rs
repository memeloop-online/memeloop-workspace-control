use sqlx::{PgConnection, PgPool, SqliteConnection, SqlitePool};

use super::{
    ConfirmPluginInstall, Database, PluginInstallInspection, StorageError, bump_postgres,
    bump_sqlite, replace_assets_postgres, replace_assets_sqlite,
};

const MAX_INSTALLED_PLUGINS: i64 = 32;

pub(super) async fn confirm_sqlite(
    pool: &SqlitePool,
    installation: &str,
    inspection: &PluginInstallInspection,
    command: &ConfirmPluginInstall<'_>,
    approved: &str,
    expected: i64,
) -> Result<(), StorageError> {
    let mut transaction = pool.begin().await?;
    if expected == 0 {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM plugin_packages WHERE installation_id=?1")
                .bind(installation)
                .fetch_one(&mut *transaction)
                .await?;
        if count >= MAX_INSTALLED_PLUGINS {
            return Err(StorageError::PluginCapacityExceeded);
        }
    }
    let changed = install_sqlite(
        &mut transaction,
        installation,
        inspection,
        command,
        approved,
        expected,
    )
    .await?;
    if changed != 1 {
        return Err(StorageError::PluginPackageVersionConflict);
    }
    replace_assets_sqlite(
        &mut transaction,
        installation,
        &inspection.plugin_id,
        &inspection.assets,
    )
    .await?;
    bump_sqlite(&mut transaction, installation).await?;
    delete_inspection_sqlite(&mut transaction, installation, command.inspection_id).await?;
    transaction.commit().await?;
    Ok(())
}

pub(super) async fn confirm_postgres(
    pool: &PgPool,
    installation: &str,
    inspection: &PluginInstallInspection,
    command: &ConfirmPluginInstall<'_>,
    approved: &str,
    expected: i64,
) -> Result<(), StorageError> {
    let mut transaction = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(installation)
        .execute(&mut *transaction)
        .await?;
    if expected == 0 {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM plugin_packages WHERE installation_id=$1")
                .bind(installation)
                .fetch_one(&mut *transaction)
                .await?;
        if count >= MAX_INSTALLED_PLUGINS {
            return Err(StorageError::PluginCapacityExceeded);
        }
    }
    let changed = install_postgres(
        &mut transaction,
        installation,
        inspection,
        command,
        approved,
        expected,
    )
    .await?;
    if changed != 1 {
        return Err(StorageError::PluginPackageVersionConflict);
    }
    replace_assets_postgres(
        &mut transaction,
        installation,
        &inspection.plugin_id,
        &inspection.assets,
    )
    .await?;
    bump_postgres(&mut transaction, installation).await?;
    delete_inspection_postgres(&mut transaction, installation, command.inspection_id).await?;
    transaction.commit().await?;
    Ok(())
}

async fn delete_inspection_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    inspection_id: uuid::Uuid,
) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM plugin_install_inspections WHERE installation_id = ?1 AND id = ?2")
        .bind(installation)
        .bind(inspection_id.to_string())
        .execute(connection)
        .await?;
    Ok(())
}

async fn delete_inspection_postgres(
    connection: &mut PgConnection,
    installation: &str,
    inspection_id: uuid::Uuid,
) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM plugin_install_inspections WHERE installation_id = $1 AND id = $2")
        .bind(installation)
        .bind(inspection_id.to_string())
        .execute(connection)
        .await?;
    Ok(())
}

pub(super) async fn mutate_package(
    database: &Database,
    plugin_id: &str,
    expected_version: u64,
    now: i64,
    enabled: Option<bool>,
) -> Result<(), StorageError> {
    let expected =
        i64::try_from(expected_version).map_err(|_| StorageError::PluginPackageVersionConflict)?;
    let changed = match database {
        Database::Sqlite {
            pool,
            installation_id,
        } => {
            let mut tx = pool.begin().await?;
            let changed = match enabled {
                Some(enabled) => sqlx::query(
                    "UPDATE plugin_packages SET enabled=?1,version=version+1,updated_at=?2 \
                     WHERE installation_id=?3 AND plugin_id=?4 AND version=?5",
                )
                .bind(i64::from(enabled))
                .bind(now)
                .bind(installation_id.as_str())
                .bind(plugin_id)
                .bind(expected)
                .execute(&mut *tx)
                .await?
                .rows_affected(),
                None => {
                    sqlx::query(
                        "DELETE FROM plugin_configurations \
                         WHERE installation_id=?1 AND plugin_id=?2",
                    )
                    .bind(installation_id.as_str())
                    .bind(plugin_id)
                    .execute(&mut *tx)
                    .await?;
                    sqlx::query(
                        "DELETE FROM plugin_packages \
                         WHERE installation_id=?1 AND plugin_id=?2 AND version=?3",
                    )
                    .bind(installation_id.as_str())
                    .bind(plugin_id)
                    .bind(expected)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected()
                }
            };
            if changed == 1 {
                bump_sqlite(&mut tx, installation_id.as_str()).await?;
                tx.commit().await?;
            }
            changed
        }
        Database::Postgres {
            pool,
            installation_id,
        } => {
            let mut tx = pool.begin().await?;
            let changed = match enabled {
                Some(enabled) => sqlx::query(
                    "UPDATE plugin_packages SET enabled=$1,version=version+1,updated_at=$2 \
                     WHERE installation_id=$3 AND plugin_id=$4 AND version=$5",
                )
                .bind(i64::from(enabled))
                .bind(now)
                .bind(installation_id.as_str())
                .bind(plugin_id)
                .bind(expected)
                .execute(&mut *tx)
                .await?
                .rows_affected(),
                None => {
                    sqlx::query(
                        "DELETE FROM plugin_configurations \
                         WHERE installation_id=$1 AND plugin_id=$2",
                    )
                    .bind(installation_id.as_str())
                    .bind(plugin_id)
                    .execute(&mut *tx)
                    .await?;
                    sqlx::query(
                        "DELETE FROM plugin_packages \
                         WHERE installation_id=$1 AND plugin_id=$2 AND version=$3",
                    )
                    .bind(installation_id.as_str())
                    .bind(plugin_id)
                    .bind(expected)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected()
                }
            };
            if changed == 1 {
                bump_postgres(&mut tx, installation_id.as_str()).await?;
                tx.commit().await?;
            }
            changed
        }
    };
    if changed != 1 {
        return Err(StorageError::PluginPackageVersionConflict);
    }
    Ok(())
}

pub(super) async fn install_sqlite(
    tx: &mut SqliteConnection,
    installation: &str,
    item: &PluginInstallInspection,
    command: &ConfirmPluginInstall<'_>,
    approved: &str,
    expected: i64,
) -> Result<u64, StorageError> {
    let changed = if expected == 0 {
        sqlx::query(
            "INSERT INTO plugin_packages \
             (installation_id,plugin_id,manifest_json,component_bytes,package_digest,source_kind,source_ref,source_confirmation,enabled,approved_contributions_json,version,created_by,created_at,updated_at) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,1,?11,?12,?12) \
             ON CONFLICT (installation_id,plugin_id) DO NOTHING",
        )
        .bind(installation)
        .bind(&item.plugin_id)
        .bind(&item.manifest_json)
        .bind(&item.component_bytes)
        .bind(&item.package_digest)
        .bind(&item.source_kind)
        .bind(&item.source_ref)
        .bind(&item.source_confirmation)
        .bind(i64::from(command.enabled))
        .bind(approved)
        .bind(command.actor_user_id.to_string())
        .bind(command.now)
        .execute(tx)
        .await?
        .rows_affected()
    } else {
        sqlx::query(
            "UPDATE plugin_packages SET \
             manifest_json=?1,component_bytes=?2,package_digest=?3,source_kind=?4,source_ref=?5,source_confirmation=?6,enabled=?7,approved_contributions_json=?8,version=version+1,updated_at=?9 \
             WHERE installation_id=?10 AND plugin_id=?11 AND version=?12",
        )
        .bind(&item.manifest_json)
        .bind(&item.component_bytes)
        .bind(&item.package_digest)
        .bind(&item.source_kind)
        .bind(&item.source_ref)
        .bind(&item.source_confirmation)
        .bind(i64::from(command.enabled))
        .bind(approved)
        .bind(command.now)
        .bind(installation)
        .bind(&item.plugin_id)
        .bind(expected)
        .execute(tx)
        .await?
        .rows_affected()
    };
    Ok(changed)
}

pub(super) async fn install_postgres(
    tx: &mut PgConnection,
    installation: &str,
    item: &PluginInstallInspection,
    command: &ConfirmPluginInstall<'_>,
    approved: &str,
    expected: i64,
) -> Result<u64, StorageError> {
    let changed = if expected == 0 {
        sqlx::query(
            "INSERT INTO plugin_packages \
             (installation_id,plugin_id,manifest_json,component_bytes,package_digest,source_kind,source_ref,source_confirmation,enabled,approved_contributions_json,version,created_by,created_at,updated_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,1,$11,$12,$12) \
             ON CONFLICT (installation_id,plugin_id) DO NOTHING",
        )
        .bind(installation)
        .bind(&item.plugin_id)
        .bind(&item.manifest_json)
        .bind(&item.component_bytes)
        .bind(&item.package_digest)
        .bind(&item.source_kind)
        .bind(&item.source_ref)
        .bind(&item.source_confirmation)
        .bind(i64::from(command.enabled))
        .bind(approved)
        .bind(command.actor_user_id.to_string())
        .bind(command.now)
        .execute(tx)
        .await?
        .rows_affected()
    } else {
        sqlx::query(
            "UPDATE plugin_packages SET \
             manifest_json=$1,component_bytes=$2,package_digest=$3,source_kind=$4,source_ref=$5,source_confirmation=$6,enabled=$7,approved_contributions_json=$8,version=version+1,updated_at=$9 \
             WHERE installation_id=$10 AND plugin_id=$11 AND version=$12",
        )
        .bind(&item.manifest_json)
        .bind(&item.component_bytes)
        .bind(&item.package_digest)
        .bind(&item.source_kind)
        .bind(&item.source_ref)
        .bind(&item.source_confirmation)
        .bind(i64::from(command.enabled))
        .bind(approved)
        .bind(command.now)
        .bind(installation)
        .bind(&item.plugin_id)
        .bind(expected)
        .execute(tx)
        .await?
        .rows_affected()
    };
    Ok(changed)
}
