use std::sync::Arc;

use axum::{
    Json,
    extract::{Multipart, Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    plugin_distribution::{
        PreparedPluginPackage, decode_bundle, download_github_release, download_https,
        sanitized_source_ref,
    },
    storage::{ConfirmPluginInstall, StorePluginInspection},
};

use super::super::{
    ApiError, AppState,
    auth::principal,
    idempotency::{hash, idempotency_key, unix_timestamp},
};

const INSPECTION_TTL_SECONDS: i64 = 15 * 60;
mod operations;
mod view;
use operations::{abandon, finish_json, reserve, system_admin};
use view::{
    PluginInspectionView, PluginPackageView, current_view, inspection_manifest, package_view,
};
#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct InspectUrlRequest {
    url: String,
    expected_sha256: String,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct InspectGithubReleaseRequest {
    repository: String,
    tag: String,
    asset: String,
    expected_sha256: String,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConfirmInstallRequest {
    inspection_id: Uuid,
    expected_digest: String,
    expected_package_version: u64,
    approved_contributions: Vec<String>,
    enabled: bool,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetEnabledRequest {
    enabled: bool,
    expected_version: u64,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UninstallRequest {
    expected_version: u64,
}

#[utoipa::path(get, path = "/api/v1/plugins", responses((status = 200, body = [PluginPackageView]), (status = 401, body = super::super::ErrorEnvelope)))]
pub(crate) async fn list_packages(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<PluginPackageView>>, ApiError> {
    let _actor = principal(&state, &headers).await?;
    state.plugins.synchronize().await?;
    Ok(Json(
        state
            .plugins
            .plugin_views()?
            .into_iter()
            .map(package_view)
            .collect(),
    ))
}

pub(crate) async fn inspect_upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Response, ApiError> {
    let actor = system_admin(&state, &headers).await?;
    let mut manifest = None;
    let mut component = None;
    let mut assets = Vec::new();
    let mut filename = "plugin.mwcpkg".to_owned();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| ApiError::PluginDistribution("plugin_package_invalid"))?
    {
        let name = field.name().unwrap_or_default().to_owned();
        let field_filename = field.file_name().map(str::to_owned);
        let media_type = field.content_type().map(str::to_owned);
        let bytes = field
            .bytes()
            .await
            .map_err(|_| ApiError::PluginDistribution("plugin_package_invalid"))?;
        match name.as_str() {
            "manifest" if manifest.is_none() => {
                filename = field_filename.unwrap_or(filename);
                manifest = Some(bytes.to_vec());
            }
            "component" if component.is_none() => component = Some(bytes.to_vec()),
            "asset" => {
                let path =
                    field_filename.ok_or(ApiError::PluginDistribution("plugin_package_invalid"))?;
                let media = media_type
                    .or_else(|| mime_guess::from_path(&path).first_raw().map(str::to_owned))
                    .ok_or(ApiError::PluginDistribution("plugin_package_invalid"))?;
                assets.push((path, media, bytes.to_vec()));
            }
            _ => return Err(ApiError::PluginDistribution("plugin_package_invalid")),
        }
    }
    let package = PreparedPluginPackage::prepare(
        manifest.ok_or(ApiError::PluginDistribution("plugin_package_invalid"))?,
        component,
        assets,
    )
    .map_err(ApiError::PluginDistribution)?;
    inspect_prepared(
        &state,
        &headers,
        actor.user_id,
        package,
        "file",
        filename,
        "administrator_confirmed",
    )
    .await
}

pub(crate) async fn inspect_url(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<InspectUrlRequest>,
) -> Result<Response, ApiError> {
    let actor = system_admin(&state, &headers).await?;
    let bytes = download_https(&request.url, &request.expected_sha256)
        .await
        .map_err(ApiError::PluginDistribution)?;
    let package = decode_bundle(&bytes).map_err(ApiError::PluginDistribution)?;
    let source_ref = sanitized_source_ref(&request.url).map_err(ApiError::PluginDistribution)?;
    inspect_prepared(
        &state,
        &headers,
        actor.user_id,
        package,
        "url",
        source_ref,
        "administrator_confirmed",
    )
    .await
}

pub(crate) async fn inspect_github_release(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<InspectGithubReleaseRequest>,
) -> Result<Response, ApiError> {
    let actor = system_admin(&state, &headers).await?;
    let bytes = download_github_release(
        &request.repository,
        &request.tag,
        &request.asset,
        &request.expected_sha256,
    )
    .await
    .map_err(ApiError::PluginDistribution)?;
    let package = decode_bundle(&bytes).map_err(ApiError::PluginDistribution)?;
    let source = serde_json::json!({"repository":request.repository,"tag":request.tag,"asset":request.asset}).to_string();
    inspect_prepared(
        &state,
        &headers,
        actor.user_id,
        package,
        "github_release",
        source,
        "administrator_confirmed",
    )
    .await
}

pub(crate) async fn confirm_install(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ConfirmInstallRequest>,
) -> Result<Response, ApiError> {
    let actor = system_admin(&state, &headers).await?;
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&request)?;
    let scope = format!("{}:plugin-install", actor.user_id);
    let now = unix_timestamp()?;
    if let Some(response) = reserve(&state, &scope, key, &request_hash, now).await? {
        return Ok(response);
    }
    let result = state
        .database
        .confirm_plugin_install(ConfirmPluginInstall {
            inspection_id: request.inspection_id,
            expected_digest: &request.expected_digest,
            expected_package_version: request.expected_package_version,
            approved_contributions: &request.approved_contributions,
            enabled: request.enabled,
            actor_user_id: actor.user_id,
            now,
        })
        .await;
    let package = match result {
        Ok(package) => package,
        Err(error) => {
            abandon(&state, &scope, key, &request_hash).await?;
            return Err(error.into());
        }
    };
    if state.plugins.force_synchronize().await.is_err() {
        abandon(&state, &scope, key, &request_hash).await?;
        return Err(ApiError::PluginDistribution("plugin_runtime_reload_failed"));
    }
    state.database.record_audit(Some(actor.user_id), None, None, "plugin.install", serde_json::json!({"plugin_id": package.plugin_id, "package_digest": package.package_digest, "enabled": package.enabled}), now).await?;
    let view = current_view(&state, &package.plugin_id)?;
    finish_json(&state, &scope, key, &request_hash, StatusCode::OK, &view).await
}

pub(crate) async fn set_enabled(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(plugin_id): Path<String>,
    Json(request): Json<SetEnabledRequest>,
) -> Result<Response, ApiError> {
    let actor = system_admin(&state, &headers).await?;
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&(&plugin_id, &request))?;
    let scope = format!("{}:plugin-enable:{plugin_id}", actor.user_id);
    let now = unix_timestamp()?;
    if let Some(response) = reserve(&state, &scope, key, &request_hash, now).await? {
        return Ok(response);
    }
    if let Err(error) = state
        .database
        .set_plugin_enabled(&plugin_id, request.enabled, request.expected_version, now)
        .await
    {
        abandon(&state, &scope, key, &request_hash).await?;
        return Err(error.into());
    }
    state
        .plugins
        .force_synchronize()
        .await
        .map_err(|_| ApiError::PluginDistribution("plugin_runtime_reload_failed"))?;
    state
        .database
        .record_audit(
            Some(actor.user_id),
            None,
            None,
            "plugin.enabled.set",
            serde_json::json!({"plugin_id":plugin_id,"enabled":request.enabled}),
            now,
        )
        .await?;
    let view = current_view(&state, &plugin_id)?;
    finish_json(&state, &scope, key, &request_hash, StatusCode::OK, &view).await
}

pub(crate) async fn uninstall(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(plugin_id): Path<String>,
    Json(request): Json<UninstallRequest>,
) -> Result<Response, ApiError> {
    let actor = system_admin(&state, &headers).await?;
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&(&plugin_id, &request))?;
    let scope = format!("{}:plugin-uninstall:{plugin_id}", actor.user_id);
    let now = unix_timestamp()?;
    if let Some(response) = reserve(&state, &scope, key, &request_hash, now).await? {
        return Ok(response);
    }
    if let Err(error) = state
        .database
        .delete_plugin_package(&plugin_id, request.expected_version, now)
        .await
    {
        abandon(&state, &scope, key, &request_hash).await?;
        return Err(error.into());
    }
    state
        .plugins
        .force_synchronize()
        .await
        .map_err(|_| ApiError::PluginDistribution("plugin_runtime_reload_failed"))?;
    state
        .database
        .record_audit(
            Some(actor.user_id),
            None,
            None,
            "plugin.uninstall",
            serde_json::json!({"plugin_id":plugin_id}),
            now,
        )
        .await?;
    finish_json(
        &state,
        &scope,
        key,
        &request_hash,
        StatusCode::NO_CONTENT,
        &(),
    )
    .await
}

async fn inspect_prepared(
    state: &AppState,
    headers: &HeaderMap,
    actor: Uuid,
    package: PreparedPluginPackage,
    source_kind: &str,
    source_ref: String,
    source_confirmation: &str,
) -> Result<Response, ApiError> {
    let key = idempotency_key(headers)?;
    let request_hash = hash(&(&package.digest, source_kind, &source_ref))?;
    let scope = format!("{actor}:plugin-inspect");
    let now = unix_timestamp()?;
    if let Some(response) = reserve(state, &scope, key, &request_hash, now).await? {
        return Ok(response);
    }
    let manifest = package.manifest.clone();
    let stored = state
        .database
        .store_plugin_inspection(StorePluginInspection {
            plugin_id: manifest.id.clone(),
            manifest_json: package.manifest_json,
            component_bytes: package.component,
            package_digest: package.digest,
            size_bytes: package.size_bytes,
            source_kind: source_kind.to_owned(),
            source_ref,
            source_confirmation: source_confirmation.to_owned(),
            declared_contributions: package.declared_contributions.clone(),
            assets: package.assets,
            created_by: actor,
            now,
            expires_at: now + INSPECTION_TTL_SECONDS,
        })
        .await?;
    let current_package_version = state
        .database
        .list_plugin_packages()
        .await?
        .into_iter()
        .find(|item| item.plugin_id == stored.plugin_id)
        .map_or(0, |item| item.version);
    let view = PluginInspectionView {
        inspection_id: stored.id,
        expires_at: stored.expires_at,
        digest: stored.package_digest,
        size_bytes: stored.size_bytes,
        source_kind: stored.source_kind,
        source_ref: stored.source_ref,
        source_confirmation: stored.source_confirmation,
        manifest: inspection_manifest(manifest),
        declared_contributions: stored.declared_contributions,
        current_package_version,
    };
    finish_json(state, &scope, key, &request_hash, StatusCode::OK, &view).await
}
