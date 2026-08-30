use std::{collections::BTreeSet, time::Duration};

use wasmtime::component::Component;

use super::{LoadedPlugin, PluginRuntime, RuntimePluginView, RuntimeRegistry};
use crate::plugins::{PluginError, PluginManifest, validate_plugin_content};

impl PluginRuntime {
    pub async fn synchronize(&self) -> Result<(), PluginError> {
        let revision = self.database.plugin_catalog_revision().await?;
        if self
            .plugins
            .read()
            .map_err(|_| PluginError::RuntimeUnavailable)?
            .catalog_revision
            == revision
        {
            return Ok(());
        }
        let _guard = self.refresh_lock.lock().await;
        if self
            .plugins
            .read()
            .map_err(|_| PluginError::RuntimeUnavailable)?
            .catalog_revision
            == revision
        {
            return Ok(());
        }
        let engine = self
            .engine
            .as_ref()
            .ok_or(PluginError::RuntimeUnavailable)?;
        let packages = self.database.list_plugin_packages().await?;
        let mounted = self
            .plugins
            .read()
            .map_err(|_| PluginError::RuntimeUnavailable)?
            .plugins
            .iter()
            .filter(|plugin| plugin.source_kind == "mounted")
            .cloned()
            .collect::<Vec<_>>();
        let mut ids = mounted
            .iter()
            .map(|plugin| plugin.manifest.id.clone())
            .collect::<BTreeSet<_>>();
        let mut loaded = mounted;
        for package in packages {
            if !ids.insert(package.plugin_id.clone()) {
                return Err(PluginError::invalid(
                    "plugin id conflicts with mounted package",
                ));
            }
            let assets = self
                .database
                .plugin_assets(&package.plugin_id)
                .await?
                .into_iter()
                .map(|asset| (asset.path, (asset.media_type, asset.content)))
                .collect();
            let validated = validate_plugin_content(
                package.manifest_json.as_bytes(),
                package.component_bytes.as_deref(),
                &assets,
            )?;
            let component = if package.enabled {
                package
                    .component_bytes
                    .as_ref()
                    .map(|bytes| {
                        Component::new(engine, bytes)
                            .map_err(|_| PluginError::invalid("component could not be compiled"))
                    })
                    .transpose()?
            } else {
                None
            };
            loaded.push(LoadedPlugin {
                manifest: validated.manifest,
                component,
                configuration_schema: validated.configuration_schema,
                source_kind: package.source_kind,
                source_ref: package.source_ref,
                package_digest: package.package_digest,
                source_confirmation: package.source_confirmation,
                approved_contributions: package.approved_contributions,
                package_version: package.version,
                enabled: package.enabled,
            });
        }
        *self
            .plugins
            .write()
            .map_err(|_| PluginError::RuntimeUnavailable)? = RuntimeRegistry {
            catalog_revision: revision,
            plugins: loaded,
        };
        Ok(())
    }

    pub async fn force_synchronize(&self) -> Result<(), PluginError> {
        self.plugins
            .write()
            .map_err(|_| PluginError::RuntimeUnavailable)?
            .catalog_revision = u64::MAX;
        self.synchronize().await
    }

    pub async fn has_api_middleware(&self) -> Result<bool, PluginError> {
        Ok(self
            .plugins
            .read()
            .map_err(|_| PluginError::RuntimeUnavailable)?
            .plugins
            .iter()
            .any(|plugin| {
                plugin.enabled
                    && !plugin.manifest.api_middleware.is_empty()
                    && plugin
                        .approved_contributions
                        .iter()
                        .any(|value| value == "api_middleware")
            }))
    }

    pub fn spawn_catalog_refresher(&self) {
        let runtime = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;
                if let Err(error) = runtime.synchronize().await {
                    tracing::error!(error = %error, "plugin catalog refresh failed; retaining last-known-good registry");
                }
            }
        });
    }

    pub fn manifests(&self) -> Vec<PluginManifest> {
        self.plugins
            .read()
            .expect("plugin runtime lock poisoned")
            .plugins
            .iter()
            .map(|plugin| plugin.manifest.clone())
            .collect()
    }

    pub fn manifest(&self, plugin_id: &str) -> Option<PluginManifest> {
        self.plugins
            .read()
            .ok()?
            .plugins
            .iter()
            .find(|plugin| plugin.manifest.id == plugin_id)
            .map(|plugin| plugin.manifest.clone())
    }

    pub(crate) fn plugin_views(&self) -> Result<Vec<RuntimePluginView>, PluginError> {
        Ok(self
            .plugins
            .read()
            .map_err(|_| PluginError::RuntimeUnavailable)?
            .plugins
            .iter()
            .map(|plugin| RuntimePluginView {
                manifest: plugin.manifest.clone(),
                source_kind: plugin.source_kind.clone(),
                source_ref: plugin.source_ref.clone(),
                package_digest: plugin.package_digest.clone(),
                source_confirmation: plugin.source_confirmation.clone(),
                approved_contributions: plugin.approved_contributions.clone(),
                package_version: plugin.package_version,
                enabled: plugin.enabled,
            })
            .collect())
    }
}
