use std::collections::BTreeMap;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::Value;
use sqlx::{Row, SqlitePool};

use super::{AssetSnapshot, CatalogSnapshot, PackageSnapshot, StorageError, to_value};
use crate::plugin_distribution::sanitized_source_ref;

pub(crate) async fn export_tables(
    pool: &SqlitePool,
    installation: &str,
) -> Result<BTreeMap<String, Vec<Value>>, StorageError> {
    let packages = sqlx::query(
        "SELECT installation_id,plugin_id,manifest_json,component_bytes,package_digest,\
         source_kind,source_ref,source_confirmation,enabled,approved_contributions_json,\
         version,created_by,created_at,updated_at FROM plugin_packages \
         WHERE installation_id=?1 ORDER BY plugin_id",
    )
    .bind(installation)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| {
        let component = row.try_get::<Option<Vec<u8>>, _>("component_bytes")?;
        let source_kind = row.try_get::<String, _>("source_kind")?;
        let source_ref = row.try_get::<String, _>("source_ref")?;
        let source_ref = if source_kind == "url" {
            sanitized_source_ref(&source_ref).map_err(|_| StorageError::InvalidPluginSnapshot)?
        } else {
            source_ref
        };
        to_value(PackageSnapshot {
            installation_id: row.try_get("installation_id")?,
            plugin_id: row.try_get("plugin_id")?,
            manifest_json: row.try_get("manifest_json")?,
            component_bytes_base64: component.map(|value| STANDARD.encode(value)),
            package_digest: row.try_get("package_digest")?,
            source_kind,
            source_ref,
            source_confirmation: row.try_get("source_confirmation")?,
            enabled: row.try_get("enabled")?,
            approved_contributions_json: row.try_get("approved_contributions_json")?,
            version: row.try_get("version")?,
            created_by: row.try_get("created_by")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    })
    .collect::<Result<Vec<_>, StorageError>>()?;
    let assets = export_assets(pool, installation).await?;
    let mut catalog = export_catalog(pool, installation).await?;
    if !packages.is_empty() && catalog.is_empty() {
        catalog.push(to_value(CatalogSnapshot {
            installation_id: installation.to_owned(),
            revision: 1,
        })?);
    }
    Ok(BTreeMap::from([
        ("plugin_packages".to_owned(), packages),
        ("plugin_assets".to_owned(), assets),
        ("plugin_catalog_metadata".to_owned(), catalog),
    ]))
}

async fn export_assets(pool: &SqlitePool, installation: &str) -> Result<Vec<Value>, StorageError> {
    sqlx::query(
        "SELECT installation_id,plugin_id,asset_path,media_type,content_bytes,content_digest \
         FROM plugin_assets WHERE installation_id=?1 ORDER BY plugin_id,asset_path",
    )
    .bind(installation)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| {
        to_value(AssetSnapshot {
            installation_id: row.try_get("installation_id")?,
            plugin_id: row.try_get("plugin_id")?,
            asset_path: row.try_get("asset_path")?,
            media_type: row.try_get("media_type")?,
            content_bytes_base64: STANDARD.encode(row.try_get::<Vec<u8>, _>("content_bytes")?),
            content_digest: row.try_get("content_digest")?,
        })
    })
    .collect()
}

async fn export_catalog(pool: &SqlitePool, installation: &str) -> Result<Vec<Value>, StorageError> {
    sqlx::query(
        "SELECT installation_id,revision FROM plugin_catalog_metadata \
         WHERE installation_id=?1",
    )
    .bind(installation)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| {
        to_value(CatalogSnapshot {
            installation_id: row.try_get("installation_id")?,
            revision: row.try_get("revision")?,
        })
    })
    .collect()
}
