use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::PluginManifest;
use crate::plugins::PluginError;

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginAssetDescriptor {
    pub path: String,
    pub media_type: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginUiSurface {
    pub id: String,
    pub title: String,
    pub placement: PluginUiPlacement,
    pub entrypoint: String,
    #[serde(default)]
    pub allowed_bridge_methods: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PluginUiPlacement {
    AdminTab,
    WorkspaceDetail,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginApiRoute {
    pub id: String,
    pub title: String,
    pub permission: PluginRoutePermission,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PluginRoutePermission {
    Authenticated,
    OrganizationAdmin,
    SystemAdmin,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginApiMiddleware {
    pub id: String,
    pub phase: String,
}

pub(super) fn validate_extensions(manifest: &PluginManifest) -> Result<(), PluginError> {
    const MAX_ASSET_BYTES: u64 = 2 * 1024 * 1024;
    const MAX_TOTAL_ASSET_BYTES: u64 = 8 * 1024 * 1024;
    if manifest.assets.len() > 64
        || manifest
            .assets
            .iter()
            .map(|asset| asset.size_bytes)
            .sum::<u64>()
            > MAX_TOTAL_ASSET_BYTES
    {
        return Err(PluginError::invalid("plugin assets exceed size limits"));
    }
    let mut paths = BTreeSet::new();
    for asset in &manifest.assets {
        if asset.size_bytes > MAX_ASSET_BYTES
            || !safe_asset_path(&asset.path)
            || !paths.insert(asset.path.as_str())
            || asset.sha256.len() != 64
            || !asset.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !matches!(
                asset.media_type.as_str(),
                "text/html"
                    | "text/css"
                    | "application/javascript"
                    | "application/json"
                    | "image/png"
                    | "image/jpeg"
                    | "image/webp"
                    | "image/svg+xml"
            )
        {
            return Err(PluginError::invalid("plugin asset declaration is invalid"));
        }
    }
    let html: BTreeSet<_> = manifest
        .assets
        .iter()
        .filter(|asset| asset.media_type == "text/html")
        .map(|asset| asset.path.as_str())
        .collect();
    let mut surface_ids = BTreeSet::new();
    for surface in &manifest.ui_surfaces {
        if !safe_id(&surface.id)
            || !surface_ids.insert(surface.id.as_str())
            || !html.contains(surface.entrypoint.as_str())
            || surface.title.trim().is_empty()
            || surface.title.len() > 120
            || surface.title.chars().any(char::is_control)
            || surface
                .allowed_bridge_methods
                .iter()
                .any(|method| !matches!(method.as_str(), "theme.read" | "plugin_api.request"))
        {
            return Err(PluginError::invalid("plugin UI surface is invalid"));
        }
    }
    if manifest
        .api_routes
        .iter()
        .any(|route| !safe_id(&route.id) || route.title.trim().is_empty())
        || manifest
            .api_middleware
            .iter()
            .any(|middleware| !safe_id(&middleware.id) || middleware.phase != "before_api")
    {
        return Err(PluginError::invalid("plugin API contribution is invalid"));
    }
    Ok(())
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

fn safe_asset_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 240
        && !value.starts_with('/')
        && !value.contains(['\\', '\0'])
        && value.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        })
}
