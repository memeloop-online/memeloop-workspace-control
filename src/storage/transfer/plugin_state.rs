use std::collections::{BTreeMap, BTreeSet};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::plugins::{PluginManifest, validate_plugin_content};

use super::{DatabaseSnapshot, StorageError};

mod export;
pub(super) use export::export_tables;
#[cfg(test)]
mod tests;

const DYNAMIC_PLUGIN_SCHEMA_VERSION: i64 = 13;
const MAX_PACKAGES: usize = 32;
const MAX_ASSETS: usize = MAX_PACKAGES * 64;
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_COMPONENT_BYTES: usize = 64 * 1024 * 1024;
const MAX_ASSET_BYTES: usize = 2 * 1024 * 1024;
const MAX_PACKAGE_BYTES: usize = 80 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = MAX_PACKAGES * MAX_PACKAGE_BYTES;
const TABLES: [&str; 3] = [
    "plugin_packages",
    "plugin_assets",
    "plugin_catalog_metadata",
];
type AssetContent = (String, Vec<u8>);
type AssetsByPath = BTreeMap<String, AssetContent>;
type AssetsByPlugin = BTreeMap<String, AssetsByPath>;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PackageSnapshot {
    installation_id: String,
    plugin_id: String,
    manifest_json: String,
    component_bytes_base64: Option<String>,
    package_digest: String,
    source_kind: String,
    source_ref: String,
    source_confirmation: String,
    enabled: i64,
    approved_contributions_json: String,
    version: i64,
    created_by: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetSnapshot {
    installation_id: String,
    plugin_id: String,
    asset_path: String,
    media_type: String,
    content_bytes_base64: String,
    content_digest: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogSnapshot {
    installation_id: String,
    revision: i64,
}

pub(super) fn is_plugin_table(table: &str) -> bool {
    TABLES.contains(&table)
}

pub(super) fn prepare_import(
    snapshot: &DatabaseSnapshot,
) -> Result<Option<BTreeMap<String, Vec<Value>>>, StorageError> {
    if snapshot.schema_version < DYNAMIC_PLUGIN_SCHEMA_VERSION {
        return Ok(None);
    }
    let package_rows = required_table(snapshot, "plugin_packages")?;
    let asset_rows = required_table(snapshot, "plugin_assets")?;
    let catalog_rows = required_table(snapshot, "plugin_catalog_metadata")?;
    if package_rows.len() > MAX_PACKAGES || asset_rows.len() > MAX_ASSETS || catalog_rows.len() > 1
    {
        return Err(StorageError::InvalidPluginSnapshot);
    }
    let packages = decode_rows::<PackageSnapshot>(package_rows)?;
    let assets = decode_rows::<AssetSnapshot>(asset_rows)?;
    let mut catalog = decode_rows::<CatalogSnapshot>(catalog_rows)?;
    let mut assets_by_plugin = decode_assets(assets, &snapshot.installation_id)?;
    let mut ids = BTreeSet::new();
    let mut total_bytes = 0_usize;
    let mut normalized_packages = Vec::with_capacity(packages.len());
    for package in packages {
        let normalized = normalize_package(
            package,
            &snapshot.installation_id,
            &mut assets_by_plugin,
            &mut ids,
            &mut total_bytes,
        )?;
        normalized_packages.push(normalized);
    }
    if !assets_by_plugin.is_empty() || total_bytes > MAX_TOTAL_BYTES {
        return Err(StorageError::InvalidPluginSnapshot);
    }
    normalize_catalog(&mut catalog, &snapshot.installation_id, !ids.is_empty())?;
    let normalized_assets = normalize_assets(asset_rows)?;
    Ok(Some(BTreeMap::from([
        ("plugin_packages".to_owned(), normalized_packages),
        ("plugin_assets".to_owned(), normalized_assets),
        (
            "plugin_catalog_metadata".to_owned(),
            catalog
                .into_iter()
                .map(to_value)
                .collect::<Result<_, _>>()?,
        ),
    ])))
}

fn decode_assets(
    assets: Vec<AssetSnapshot>,
    installation: &str,
) -> Result<AssetsByPlugin, StorageError> {
    let mut grouped = AssetsByPlugin::new();
    for asset in assets {
        if asset.installation_id != installation {
            return Err(StorageError::InvalidPluginSnapshot);
        }
        let content = decode_bounded(&asset.content_bytes_base64, MAX_ASSET_BYTES)?;
        let digest = format!("{:x}", Sha256::digest(&content));
        if asset.content_digest != digest
            || grouped
                .entry(asset.plugin_id)
                .or_default()
                .insert(asset.asset_path, (asset.media_type, content))
                .is_some()
        {
            return Err(StorageError::InvalidPluginSnapshot);
        }
    }
    Ok(grouped)
}

fn normalize_package(
    package: PackageSnapshot,
    installation: &str,
    assets_by_plugin: &mut AssetsByPlugin,
    ids: &mut BTreeSet<String>,
    total_bytes: &mut usize,
) -> Result<Value, StorageError> {
    validate_package_metadata(&package, installation, ids)?;
    let component = package
        .component_bytes_base64
        .as_deref()
        .map(|value| decode_bounded(value, MAX_COMPONENT_BYTES))
        .transpose()?;
    let assets = assets_by_plugin
        .remove(&package.plugin_id)
        .unwrap_or_default();
    let validated = validate_plugin_content(
        package.manifest_json.as_bytes(),
        component.as_deref(),
        &assets,
    )
    .map_err(|_| StorageError::InvalidPluginSnapshot)?;
    if validated.manifest.id != package.plugin_id {
        return Err(StorageError::InvalidPluginSnapshot);
    }
    validate_approvals(&package.approved_contributions_json, &validated.manifest)?;
    let asset_bytes = assets.values().try_fold(0_usize, |total, (_, content)| {
        total
            .checked_add(content.len())
            .ok_or(StorageError::InvalidPluginSnapshot)
    })?;
    let package_bytes = package
        .manifest_json
        .len()
        .checked_add(component.as_ref().map_or(0, Vec::len))
        .and_then(|value| value.checked_add(asset_bytes))
        .ok_or(StorageError::InvalidPluginSnapshot)?;
    if package.manifest_json.len() > MAX_MANIFEST_BYTES || package_bytes > MAX_PACKAGE_BYTES {
        return Err(StorageError::InvalidPluginSnapshot);
    }
    *total_bytes = total_bytes
        .checked_add(package_bytes)
        .ok_or(StorageError::InvalidPluginSnapshot)?;
    Ok(serde_json::json!({
        "installation_id": package.installation_id,
        "plugin_id": package.plugin_id,
        "manifest_json": package.manifest_json,
        "component_bytes": component.as_deref().map(postgres_bytea),
        "package_digest": package.package_digest,
        "source_kind": package.source_kind,
        "source_ref": package.source_ref,
        "source_confirmation": package.source_confirmation,
        "enabled": package.enabled,
        "approved_contributions_json": package.approved_contributions_json,
        "version": package.version,
        "created_by": package.created_by,
        "created_at": package.created_at,
        "updated_at": package.updated_at,
    }))
}

fn validate_package_metadata(
    package: &PackageSnapshot,
    installation: &str,
    ids: &mut BTreeSet<String>,
) -> Result<(), StorageError> {
    let url_source_is_safe = package.source_kind != "url"
        || url::Url::parse(&package.source_ref).is_ok_and(|url| {
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && url.port().is_none()
        });
    if package.installation_id != installation
        || !ids.insert(package.plugin_id.clone())
        || package.package_digest.len() != 64
        || !package
            .package_digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || !matches!(
            package.source_confirmation.as_str(),
            "administrator_confirmed" | "gitops_mounted"
        )
        || package.source_kind.is_empty()
        || package.source_kind.len() > 64
        || package.source_kind.chars().any(char::is_control)
        || package.source_ref.len() > 2_048
        || package.source_ref.chars().any(char::is_control)
        || !url_source_is_safe
        || !matches!(package.enabled, 0 | 1)
        || package.version < 1
        || Uuid::parse_str(&package.created_by).is_err()
    {
        return Err(StorageError::InvalidPluginSnapshot);
    }
    Ok(())
}

fn validate_approvals(value: &str, manifest: &PluginManifest) -> Result<(), StorageError> {
    if value.len() > 512 {
        return Err(StorageError::InvalidPluginSnapshot);
    }
    let approved: Vec<String> =
        serde_json::from_str(value).map_err(|_| StorageError::InvalidPluginSnapshot)?;
    let mut declared = BTreeSet::new();
    if manifest.workspace_create_policy {
        declared.insert("workspace_create_policy");
    }
    if manifest.configuration.is_some() {
        declared.insert("configuration");
    }
    if !manifest.ui_surfaces.is_empty() {
        declared.insert("ui_surfaces");
    }
    if !manifest.api_routes.is_empty() {
        declared.insert("api_routes");
    }
    if !manifest.api_middleware.is_empty() {
        declared.insert("api_middleware");
    }
    let mut unique = BTreeSet::new();
    if approved
        .iter()
        .any(|item| !declared.contains(item.as_str()) || !unique.insert(item))
    {
        return Err(StorageError::InvalidPluginSnapshot);
    }
    Ok(())
}

fn normalize_assets(rows: &[Value]) -> Result<Vec<Value>, StorageError> {
    decode_rows::<AssetSnapshot>(rows)?
        .into_iter()
        .map(|asset| {
            let content = decode_bounded(&asset.content_bytes_base64, MAX_ASSET_BYTES)?;
            Ok(serde_json::json!({
                "installation_id": asset.installation_id,
                "plugin_id": asset.plugin_id,
                "asset_path": asset.asset_path,
                "media_type": asset.media_type,
                "content_bytes": postgres_bytea(&content),
                "content_digest": asset.content_digest,
            }))
        })
        .collect()
}

fn normalize_catalog(
    rows: &mut Vec<CatalogSnapshot>,
    installation: &str,
    has_packages: bool,
) -> Result<(), StorageError> {
    if rows
        .iter()
        .any(|row| row.installation_id != installation || row.revision < 0)
    {
        return Err(StorageError::InvalidPluginSnapshot);
    }
    if has_packages && rows.is_empty() {
        rows.push(CatalogSnapshot {
            installation_id: installation.to_owned(),
            revision: 1,
        });
    } else if has_packages && rows[0].revision == 0 {
        rows[0].revision = 1;
    }
    Ok(())
}

fn required_table<'a>(
    snapshot: &'a DatabaseSnapshot,
    table: &str,
) -> Result<&'a [Value], StorageError> {
    snapshot
        .tables
        .get(table)
        .map(Vec::as_slice)
        .ok_or_else(|| StorageError::SnapshotMissingTable(table.to_owned()))
}

fn decode_rows<T>(rows: &[Value]) -> Result<Vec<T>, StorageError>
where
    T: for<'de> Deserialize<'de>,
{
    rows.iter()
        .cloned()
        .map(|row| serde_json::from_value(row).map_err(|_| StorageError::InvalidPluginSnapshot))
        .collect()
}

fn decode_bounded(value: &str, maximum: usize) -> Result<Vec<u8>, StorageError> {
    let maximum_encoded = maximum.div_ceil(3).saturating_mul(4);
    if value.len() > maximum_encoded {
        return Err(StorageError::InvalidPluginSnapshot);
    }
    let decoded = STANDARD
        .decode(value)
        .map_err(|_| StorageError::InvalidPluginSnapshot)?;
    if decoded.len() > maximum {
        return Err(StorageError::InvalidPluginSnapshot);
    }
    Ok(decoded)
}

fn postgres_bytea(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(2 + value.len() * 2);
    encoded.push_str("\\x");
    for byte in value {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn to_value<T: Serialize>(value: T) -> Result<Value, StorageError> {
    serde_json::to_value(value).map_err(StorageError::from)
}
