use uuid::Uuid;

use super::{LoadedPlugin, PluginRuntime};
use crate::plugins::PluginError;

impl PluginRuntime {
    pub fn validate_configuration(
        &self,
        plugin_id: &str,
        value: &serde_json::Value,
    ) -> Result<(), PluginError> {
        let registry = self
            .plugins
            .read()
            .map_err(|_| PluginError::RuntimeUnavailable)?;
        let plugin = registry
            .plugins
            .iter()
            .find(|plugin| plugin.manifest.id == plugin_id)
            .ok_or(PluginError::NotFound)?;
        plugin
            .configuration_schema
            .as_ref()
            .ok_or(PluginError::InvalidConfiguration)?
            .validate(value)
    }

    pub fn configuration_schema_digest(&self, plugin_id: &str) -> Result<String, PluginError> {
        let registry = self
            .plugins
            .read()
            .map_err(|_| PluginError::RuntimeUnavailable)?;
        let plugin = registry
            .plugins
            .iter()
            .find(|plugin| plugin.manifest.id == plugin_id)
            .ok_or(PluginError::NotFound)?;
        Ok(plugin
            .configuration_schema
            .as_ref()
            .ok_or(PluginError::InvalidConfiguration)?
            .digest()
            .to_owned())
    }

    pub(super) async fn effective_configuration(
        &self,
        plugin: &LoadedPlugin,
        organization_id: Option<Uuid>,
    ) -> Result<serde_json::Value, PluginError> {
        let configured = self
            .database
            .plugin_configuration_for_scope(&plugin.manifest.id, organization_id)
            .await?
            .or(if organization_id.is_some() {
                self.database
                    .plugin_configuration_for_scope(&plugin.manifest.id, None)
                    .await?
            } else {
                None
            });
        if configured.as_ref().is_some_and(|stored| {
            plugin
                .configuration_schema
                .as_ref()
                .is_none_or(|schema| stored.schema_digest != schema.digest())
        }) {
            return Err(PluginError::InvalidConfiguration);
        }
        let value = configured.map_or_else(
            || {
                plugin
                    .manifest
                    .configuration
                    .as_ref()
                    .map(|configuration| configuration.default.clone())
                    .unwrap_or_else(|| serde_json::json!({}))
            },
            |stored| stored.value,
        );
        if let Some(schema) = &plugin.configuration_schema {
            schema.validate(&value)?;
        }
        Ok(value)
    }
}
