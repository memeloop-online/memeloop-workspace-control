use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginAssetBlob {
    pub path: String,
    pub media_type: String,
    pub content: Vec<u8>,
    pub digest: String,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct PluginPackageRecord {
    pub plugin_id: String,
    pub manifest_json: String,
    #[serde(skip)]
    pub component_bytes: Option<Vec<u8>>,
    pub package_digest: String,
    pub source_kind: String,
    pub source_ref: String,
    pub source_confirmation: String,
    pub enabled: bool,
    pub approved_contributions: Vec<String>,
    pub version: u64,
    pub created_by: Uuid,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct PluginInstallInspection {
    pub id: Uuid,
    pub plugin_id: String,
    pub manifest_json: String,
    #[serde(skip)]
    pub component_bytes: Option<Vec<u8>>,
    pub package_digest: String,
    pub size_bytes: u64,
    pub source_kind: String,
    pub source_ref: String,
    pub source_confirmation: String,
    pub declared_contributions: Vec<String>,
    #[serde(skip)]
    pub assets: Vec<PluginAssetBlob>,
    pub created_by: Uuid,
    pub created_at: i64,
    pub expires_at: i64,
}

pub struct StorePluginInspection {
    pub plugin_id: String,
    pub manifest_json: String,
    pub component_bytes: Option<Vec<u8>>,
    pub package_digest: String,
    pub size_bytes: u64,
    pub source_kind: String,
    pub source_ref: String,
    pub source_confirmation: String,
    pub declared_contributions: Vec<String>,
    pub assets: Vec<PluginAssetBlob>,
    pub created_by: Uuid,
    pub now: i64,
    pub expires_at: i64,
}

pub struct ConfirmPluginInstall<'a> {
    pub inspection_id: Uuid,
    pub expected_digest: &'a str,
    pub expected_package_version: u64,
    pub approved_contributions: &'a [String],
    pub enabled: bool,
    pub actor_user_id: Uuid,
    pub now: i64,
}

#[derive(Clone, Debug)]
pub struct PluginUiSession {
    pub id: Uuid,
    pub plugin_id: String,
    pub surface_id: String,
    pub user_id: Uuid,
    pub entrypoint: String,
    pub package_digest: String,
    pub channel_nonce: String,
    pub allowed_bridge_methods: Vec<String>,
    pub expires_at: i64,
}

pub struct CreatePluginUiSession<'a> {
    pub plugin_id: &'a str,
    pub surface_id: &'a str,
    pub user_id: Uuid,
    pub ticket_hash: &'a str,
    pub cookie_hash: &'a str,
    pub channel_nonce: &'a str,
    pub allowed_bridge_methods: &'a [String],
    pub entrypoint: &'a str,
    pub package_digest: &'a str,
    pub expires_at: i64,
    pub now: i64,
}
