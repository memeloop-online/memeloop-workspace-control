mod codec;
mod lifecycle;
mod types;
mod ui_sessions;

pub use types::{
    ConfirmPluginInstall, CreatePluginUiSession, PluginAssetBlob, PluginInstallInspection,
    PluginPackageRecord, PluginUiSession, StorePluginInspection,
};

use sqlx::{PgConnection, SqliteConnection};
use uuid::Uuid;

use super::{Database, StorageError};
use codec::{decode_asset, decode_inspection, decode_package, encode_assets};
use lifecycle::{confirm_postgres, confirm_sqlite, mutate_package};

impl Database {
    pub async fn plugin_catalog_revision(&self) -> Result<u64, StorageError> {
        let value: i64 = match self {
            Self::Sqlite { pool, installation_id } => sqlx::query_scalar(
                "SELECT COALESCE((SELECT revision FROM plugin_catalog_metadata WHERE installation_id = ?1), 0)",
            ).bind(installation_id.as_str()).fetch_one(pool).await?,
            Self::Postgres { pool, installation_id } => sqlx::query_scalar(
                "SELECT COALESCE((SELECT revision FROM plugin_catalog_metadata WHERE installation_id = $1), 0)",
            ).bind(installation_id.as_str()).fetch_one(pool).await?,
        };
        u64::try_from(value).map_err(|_| StorageError::InvalidPluginConfiguration)
    }

    pub async fn list_plugin_packages(&self) -> Result<Vec<PluginPackageRecord>, StorageError> {
        const COLUMNS: &str = "plugin_id, manifest_json, component_bytes, package_digest, source_kind, source_ref, source_confirmation, enabled, approved_contributions_json, version, created_by, created_at, updated_at";
        match self {
            Self::Sqlite { pool, installation_id } => sqlx::query(&format!("SELECT {COLUMNS} FROM plugin_packages WHERE installation_id = ?1 ORDER BY plugin_id"))
                .bind(installation_id.as_str()).fetch_all(pool).await?.into_iter().map(decode_package).collect(),
            Self::Postgres { pool, installation_id } => sqlx::query(&format!("SELECT {COLUMNS} FROM plugin_packages WHERE installation_id = $1 ORDER BY plugin_id"))
                .bind(installation_id.as_str()).fetch_all(pool).await?.into_iter().map(decode_package).collect(),
        }
    }

    pub async fn store_plugin_inspection(
        &self,
        mut inspection: StorePluginInspection,
    ) -> Result<PluginInstallInspection, StorageError> {
        validate_package_input(&inspection.plugin_id, &inspection.package_digest)?;
        if inspection.source_kind == "url" {
            inspection.source_ref =
                crate::plugin_distribution::sanitized_source_ref(&inspection.source_ref)
                    .map_err(|_| StorageError::InvalidPluginConfiguration)?;
        }
        let id = Uuid::now_v7();
        let declared = serde_json::to_string(&inspection.declared_contributions)?;
        let assets = encode_assets(&inspection.assets)?;
        let size = i64::try_from(inspection.size_bytes)
            .map_err(|_| StorageError::InvalidPluginConfiguration)?;
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query("DELETE FROM plugin_install_inspections WHERE installation_id = ?1 AND expires_at <= ?2")
                    .bind(installation_id.as_str()).bind(inspection.now).execute(pool).await?;
                let active: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM plugin_install_inspections WHERE installation_id=?1",
                )
                .bind(installation_id.as_str())
                .fetch_one(pool)
                .await?;
                let actor_active: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM plugin_install_inspections WHERE installation_id=?1 AND created_by=?2").bind(installation_id.as_str()).bind(inspection.created_by.to_string()).fetch_one(pool).await?;
                if active >= 8 || actor_active >= 4 {
                    return Err(StorageError::TooManyPluginInspections);
                }
                sqlx::query("INSERT INTO plugin_install_inspections (id, installation_id, plugin_id, manifest_json, component_bytes, package_digest, size_bytes, source_kind, source_ref, source_confirmation, declared_contributions_json, assets_json, created_by, created_at, expires_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)")
                    .bind(id.to_string()).bind(installation_id.as_str()).bind(&inspection.plugin_id)
                    .bind(&inspection.manifest_json).bind(&inspection.component_bytes).bind(&inspection.package_digest)
                    .bind(size).bind(&inspection.source_kind).bind(&inspection.source_ref).bind(&inspection.source_confirmation)
                    .bind(declared).bind(assets).bind(inspection.created_by.to_string()).bind(inspection.now).bind(inspection.expires_at)
                    .execute(pool).await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query("DELETE FROM plugin_install_inspections WHERE installation_id = $1 AND expires_at <= $2")
                    .bind(installation_id.as_str()).bind(inspection.now).execute(pool).await?;
                let active: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM plugin_install_inspections WHERE installation_id=$1",
                )
                .bind(installation_id.as_str())
                .fetch_one(pool)
                .await?;
                let actor_active: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM plugin_install_inspections WHERE installation_id=$1 AND created_by=$2").bind(installation_id.as_str()).bind(inspection.created_by.to_string()).fetch_one(pool).await?;
                if active >= 8 || actor_active >= 4 {
                    return Err(StorageError::TooManyPluginInspections);
                }
                sqlx::query("INSERT INTO plugin_install_inspections (id, installation_id, plugin_id, manifest_json, component_bytes, package_digest, size_bytes, source_kind, source_ref, source_confirmation, declared_contributions_json, assets_json, created_by, created_at, expires_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)")
                    .bind(id.to_string()).bind(installation_id.as_str()).bind(&inspection.plugin_id)
                    .bind(&inspection.manifest_json).bind(&inspection.component_bytes).bind(&inspection.package_digest)
                    .bind(size).bind(&inspection.source_kind).bind(&inspection.source_ref).bind(&inspection.source_confirmation)
                    .bind(declared).bind(assets).bind(inspection.created_by.to_string()).bind(inspection.now).bind(inspection.expires_at)
                    .execute(pool).await?;
            }
        }
        Ok(PluginInstallInspection {
            id,
            plugin_id: inspection.plugin_id,
            manifest_json: inspection.manifest_json,
            component_bytes: inspection.component_bytes,
            package_digest: inspection.package_digest,
            size_bytes: inspection.size_bytes,
            source_kind: inspection.source_kind,
            source_ref: inspection.source_ref,
            source_confirmation: inspection.source_confirmation,
            declared_contributions: inspection.declared_contributions,
            assets: inspection.assets,
            created_by: inspection.created_by,
            created_at: inspection.now,
            expires_at: inspection.expires_at,
        })
    }

    pub async fn get_plugin_inspection(
        &self,
        id: Uuid,
    ) -> Result<PluginInstallInspection, StorageError> {
        const COLUMNS: &str = "id, plugin_id, manifest_json, component_bytes, package_digest, size_bytes, source_kind, source_ref, source_confirmation, declared_contributions_json, assets_json, created_by, created_at, expires_at";
        match self {
            Self::Sqlite { pool, installation_id } => sqlx::query(&format!("SELECT {COLUMNS} FROM plugin_install_inspections WHERE installation_id = ?1 AND id = ?2"))
                .bind(installation_id.as_str()).bind(id.to_string()).fetch_optional(pool).await?.map(decode_inspection).transpose()?.ok_or(StorageError::PluginInspectionNotFound),
            Self::Postgres { pool, installation_id } => sqlx::query(&format!("SELECT {COLUMNS} FROM plugin_install_inspections WHERE installation_id = $1 AND id = $2"))
                .bind(installation_id.as_str()).bind(id.to_string()).fetch_optional(pool).await?.map(decode_inspection).transpose()?.ok_or(StorageError::PluginInspectionNotFound),
        }
    }

    pub async fn confirm_plugin_install(
        &self,
        command: ConfirmPluginInstall<'_>,
    ) -> Result<PluginPackageRecord, StorageError> {
        let inspection = self.get_plugin_inspection(command.inspection_id).await?;
        if inspection.expires_at <= command.now {
            return Err(StorageError::PluginInspectionExpired);
        }
        if inspection.package_digest != command.expected_digest {
            return Err(StorageError::PluginDigestMismatch);
        }
        if command
            .approved_contributions
            .iter()
            .any(|approved| !inspection.declared_contributions.contains(approved))
        {
            return Err(StorageError::PluginCapabilityNotApproved);
        }
        let approved = serde_json::to_string(command.approved_contributions)?;
        let expected = i64::try_from(command.expected_package_version)
            .map_err(|_| StorageError::PluginPackageVersionConflict)?;
        let next = expected.saturating_add(1);
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                confirm_sqlite(
                    pool,
                    installation_id.as_str(),
                    &inspection,
                    &command,
                    &approved,
                    expected,
                )
                .await?
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                confirm_postgres(
                    pool,
                    installation_id.as_str(),
                    &inspection,
                    &command,
                    &approved,
                    expected,
                )
                .await?
            }
        }
        Ok(PluginPackageRecord {
            plugin_id: inspection.plugin_id,
            manifest_json: inspection.manifest_json,
            component_bytes: inspection.component_bytes,
            package_digest: inspection.package_digest,
            source_kind: inspection.source_kind,
            source_ref: inspection.source_ref,
            source_confirmation: inspection.source_confirmation,
            enabled: command.enabled,
            approved_contributions: command.approved_contributions.to_vec(),
            version: u64::try_from(next).map_err(|_| StorageError::PluginPackageVersionConflict)?,
            created_by: command.actor_user_id,
            created_at: command.now,
            updated_at: command.now,
        })
    }

    pub async fn set_plugin_enabled(
        &self,
        plugin_id: &str,
        enabled: bool,
        expected_version: u64,
        now: i64,
    ) -> Result<(), StorageError> {
        mutate_package(self, plugin_id, expected_version, now, Some(enabled)).await
    }

    pub async fn delete_plugin_package(
        &self,
        plugin_id: &str,
        expected_version: u64,
        now: i64,
    ) -> Result<(), StorageError> {
        mutate_package(self, plugin_id, expected_version, now, None).await
    }

    pub async fn plugin_asset(
        &self,
        plugin_id: &str,
        path: &str,
    ) -> Result<Option<PluginAssetBlob>, StorageError> {
        match self {
            Self::Sqlite { pool, installation_id } => sqlx::query("SELECT asset_path, media_type, content_bytes, content_digest FROM plugin_assets WHERE installation_id = ?1 AND plugin_id = ?2 AND asset_path = ?3")
                .bind(installation_id.as_str()).bind(plugin_id).bind(path).fetch_optional(pool).await?.map(decode_asset).transpose(),
            Self::Postgres { pool, installation_id } => sqlx::query("SELECT asset_path, media_type, content_bytes, content_digest FROM plugin_assets WHERE installation_id = $1 AND plugin_id = $2 AND asset_path = $3")
                .bind(installation_id.as_str()).bind(plugin_id).bind(path).fetch_optional(pool).await?.map(decode_asset).transpose(),
        }
    }

    pub async fn plugin_assets(
        &self,
        plugin_id: &str,
    ) -> Result<Vec<PluginAssetBlob>, StorageError> {
        match self {
            Self::Sqlite { pool, installation_id } => sqlx::query("SELECT asset_path, media_type, content_bytes, content_digest FROM plugin_assets WHERE installation_id = ?1 AND plugin_id = ?2 ORDER BY asset_path")
                .bind(installation_id.as_str()).bind(plugin_id).fetch_all(pool).await?.into_iter().map(decode_asset).collect(),
            Self::Postgres { pool, installation_id } => sqlx::query("SELECT asset_path, media_type, content_bytes, content_digest FROM plugin_assets WHERE installation_id = $1 AND plugin_id = $2 ORDER BY asset_path")
                .bind(installation_id.as_str()).bind(plugin_id).fetch_all(pool).await?.into_iter().map(decode_asset).collect(),
        }
    }
}

pub(super) async fn replace_assets_sqlite(
    tx: &mut SqliteConnection,
    installation: &str,
    plugin_id: &str,
    assets: &[PluginAssetBlob],
) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM plugin_assets WHERE installation_id = ?1 AND plugin_id = ?2")
        .bind(installation)
        .bind(plugin_id)
        .execute(&mut *tx)
        .await?;
    for asset in assets {
        sqlx::query("INSERT INTO plugin_assets (installation_id, plugin_id, asset_path, media_type, content_bytes, content_digest) VALUES (?1,?2,?3,?4,?5,?6)")
            .bind(installation).bind(plugin_id).bind(&asset.path).bind(&asset.media_type).bind(&asset.content).bind(&asset.digest).execute(&mut *tx).await?;
    }
    Ok(())
}

pub(super) async fn replace_assets_postgres(
    tx: &mut PgConnection,
    installation: &str,
    plugin_id: &str,
    assets: &[PluginAssetBlob],
) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM plugin_assets WHERE installation_id = $1 AND plugin_id = $2")
        .bind(installation)
        .bind(plugin_id)
        .execute(&mut *tx)
        .await?;
    for asset in assets {
        sqlx::query("INSERT INTO plugin_assets (installation_id, plugin_id, asset_path, media_type, content_bytes, content_digest) VALUES ($1,$2,$3,$4,$5,$6)")
            .bind(installation).bind(plugin_id).bind(&asset.path).bind(&asset.media_type).bind(&asset.content).bind(&asset.digest).execute(&mut *tx).await?;
    }
    Ok(())
}

pub(super) async fn bump_sqlite(
    tx: &mut SqliteConnection,
    installation: &str,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO plugin_catalog_metadata (installation_id, revision) VALUES (?1,1) ON CONFLICT (installation_id) DO UPDATE SET revision = revision + 1")
        .bind(installation).execute(tx).await?;
    Ok(())
}

pub(super) async fn bump_postgres(
    tx: &mut PgConnection,
    installation: &str,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO plugin_catalog_metadata (installation_id, revision) VALUES ($1,1) ON CONFLICT (installation_id) DO UPDATE SET revision = plugin_catalog_metadata.revision + 1")
        .bind(installation).execute(tx).await?;
    Ok(())
}

fn validate_package_input(plugin_id: &str, digest: &str) -> Result<(), StorageError> {
    if plugin_id.is_empty()
        || plugin_id.len() > 64
        || !plugin_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || digest.len() != 64
        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(StorageError::InvalidPluginConfiguration);
    }
    Ok(())
}
