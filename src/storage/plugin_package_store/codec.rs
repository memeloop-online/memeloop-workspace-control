use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use super::{PluginAssetBlob, PluginInstallInspection, PluginPackageRecord, StorageError};

pub(super) fn decode_package<R: Row>(row: R) -> Result<PluginPackageRecord, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    Vec<u8>: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    Ok(PluginPackageRecord {
        plugin_id: row.try_get("plugin_id")?,
        manifest_json: row.try_get("manifest_json")?,
        component_bytes: row.try_get("component_bytes")?,
        package_digest: row.try_get("package_digest")?,
        source_kind: row.try_get("source_kind")?,
        source_ref: row.try_get("source_ref")?,
        source_confirmation: row.try_get("source_confirmation")?,
        enabled: row.try_get::<i64, _>("enabled")? != 0,
        approved_contributions: serde_json::from_str(
            &row.try_get::<String, _>("approved_contributions_json")?,
        )?,
        version: u64::try_from(row.try_get::<i64, _>("version")?)
            .map_err(|_| StorageError::InvalidPluginConfiguration)?,
        created_by: Uuid::parse_str(&row.try_get::<String, _>("created_by")?)?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

pub(super) fn decode_inspection<R: Row>(row: R) -> Result<PluginInstallInspection, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    Vec<u8>: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    Ok(PluginInstallInspection {
        id: Uuid::parse_str(&row.try_get::<String, _>("id")?)?,
        plugin_id: row.try_get("plugin_id")?,
        manifest_json: row.try_get("manifest_json")?,
        component_bytes: row.try_get("component_bytes")?,
        package_digest: row.try_get("package_digest")?,
        size_bytes: u64::try_from(row.try_get::<i64, _>("size_bytes")?)
            .map_err(|_| StorageError::InvalidPluginConfiguration)?,
        source_kind: row.try_get("source_kind")?,
        source_ref: row.try_get("source_ref")?,
        source_confirmation: row.try_get("source_confirmation")?,
        declared_contributions: serde_json::from_str(
            &row.try_get::<String, _>("declared_contributions_json")?,
        )?,
        assets: decode_assets(&row.try_get::<String, _>("assets_json")?)?,
        created_by: Uuid::parse_str(&row.try_get::<String, _>("created_by")?)?,
        created_at: row.try_get("created_at")?,
        expires_at: row.try_get("expires_at")?,
    })
}

pub(super) fn decode_asset<R: Row>(row: R) -> Result<PluginAssetBlob, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    Vec<u8>: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    Ok(PluginAssetBlob {
        path: row.try_get("asset_path")?,
        media_type: row.try_get("media_type")?,
        content: row.try_get("content_bytes")?,
        digest: row.try_get("content_digest")?,
    })
}

#[derive(Serialize, Deserialize)]
struct EncodedAsset {
    path: String,
    media_type: String,
    content_base64: String,
    digest: String,
}

pub(super) fn encode_assets(assets: &[PluginAssetBlob]) -> Result<String, StorageError> {
    serde_json::to_string(
        &assets
            .iter()
            .map(|asset| EncodedAsset {
                path: asset.path.clone(),
                media_type: asset.media_type.clone(),
                content_base64: STANDARD.encode(&asset.content),
                digest: asset.digest.clone(),
            })
            .collect::<Vec<_>>(),
    )
    .map_err(StorageError::from)
}

fn decode_assets(value: &str) -> Result<Vec<PluginAssetBlob>, StorageError> {
    serde_json::from_str::<Vec<EncodedAsset>>(value)?
        .into_iter()
        .map(|asset| {
            Ok(PluginAssetBlob {
                path: asset.path,
                media_type: asset.media_type,
                content: STANDARD
                    .decode(asset.content_base64)
                    .map_err(|_| StorageError::InvalidPluginConfiguration)?,
                digest: asset.digest,
            })
        })
        .collect()
}
