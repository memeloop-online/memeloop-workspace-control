use serde::Serialize;
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use super::super::super::{ApiError, AppState};
use crate::plugins::{PluginManifest, RuntimePluginView};

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct PluginPackageView {
    id: String,
    name: String,
    version: String,
    description: String,
    wit_version: String,
    workspace_create_policy: bool,
    denial_codes: Vec<String>,
    declared_contributions: Vec<String>,
    approved_contributions: Vec<String>,
    configuration_schema: Option<Value>,
    configuration_default: Option<Value>,
    package_digest: String,
    source_kind: String,
    source_ref: String,
    source_details: Value,
    source_confirmation: String,
    enabled: bool,
    package_version: u64,
    runtime_status: &'static str,
    runtime_error_code: Option<&'static str>,
    ui_surfaces: Vec<crate::plugins::PluginUiSurface>,
    api_routes: Vec<String>,
    api_middleware: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct PluginInspectionView {
    pub(super) inspection_id: Uuid,
    pub(super) expires_at: i64,
    pub(super) digest: String,
    pub(super) size_bytes: u64,
    pub(super) source_kind: String,
    pub(super) source_ref: String,
    pub(super) source_confirmation: String,
    pub(super) manifest: PluginInspectionManifest,
    pub(super) declared_contributions: Vec<String>,
    pub(super) current_package_version: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct PluginInspectionManifest {
    id: String,
    name: String,
    version: String,
    description: String,
    wit_version: String,
}

pub(super) fn package_view(plugin: RuntimePluginView) -> PluginPackageView {
    let manifest = plugin.manifest;
    let declared = declared(&manifest);
    let (configuration_schema, configuration_default) = manifest
        .configuration
        .as_ref()
        .map_or((None, None), |value| {
            (Some(value.schema.clone()), Some(value.default.clone()))
        });
    PluginPackageView {
        id: manifest.id,
        name: manifest.name,
        version: manifest.version,
        description: manifest.description,
        wit_version: manifest.wit_version,
        workspace_create_policy: manifest.workspace_create_policy,
        denial_codes: manifest.denial_codes,
        declared_contributions: declared,
        approved_contributions: plugin.approved_contributions,
        configuration_schema,
        configuration_default,
        package_digest: plugin.package_digest,
        source_kind: plugin.source_kind.clone(),
        source_ref: plugin.source_ref.clone(),
        source_details: source_details(&plugin.source_kind, &plugin.source_ref),
        source_confirmation: plugin.source_confirmation,
        enabled: plugin.enabled,
        package_version: plugin.package_version,
        runtime_status: if plugin.enabled { "loaded" } else { "disabled" },
        runtime_error_code: None,
        ui_surfaces: manifest.ui_surfaces,
        api_routes: manifest
            .api_routes
            .into_iter()
            .map(|route| route.id)
            .collect(),
        api_middleware: manifest
            .api_middleware
            .into_iter()
            .map(|middleware| middleware.id)
            .collect(),
    }
}
fn source_details(kind: &str, source: &str) -> Value {
    match kind{"url"=>serde_json::json!({"kind":"url","url":source}),"github_release"=>serde_json::from_str::<Value>(source).map(|mut value|{if let Some(object)=value.as_object_mut(){object.insert("kind".to_owned(),Value::String("github_release".to_owned()));}value}).unwrap_or_else(|_|serde_json::json!({"kind":"github_release","repository":"","tag":"","asset":""})),"mounted"=>serde_json::json!({"kind":"mounted","name":source}),_=>serde_json::json!({"kind":"file","filename":source})}
}
fn declared(manifest: &PluginManifest) -> Vec<String> {
    [
        (manifest.workspace_create_policy, "workspace_create_policy"),
        (manifest.configuration.is_some(), "configuration"),
        (!manifest.ui_surfaces.is_empty(), "ui_surfaces"),
        (!manifest.api_routes.is_empty(), "api_routes"),
        (!manifest.api_middleware.is_empty(), "api_middleware"),
    ]
    .into_iter()
    .filter(|item| item.0)
    .map(|item| item.1.to_owned())
    .collect()
}
pub(super) fn inspection_manifest(manifest: PluginManifest) -> PluginInspectionManifest {
    PluginInspectionManifest {
        id: manifest.id,
        name: manifest.name,
        version: manifest.version,
        description: manifest.description,
        wit_version: manifest.wit_version,
    }
}
pub(super) fn current_view(state: &AppState, id: &str) -> Result<PluginPackageView, ApiError> {
    state
        .plugins
        .plugin_views()?
        .into_iter()
        .find(|item| item.manifest.id == id)
        .map(package_view)
        .ok_or(ApiError::PluginDistribution("plugin_runtime_reload_failed"))
}
