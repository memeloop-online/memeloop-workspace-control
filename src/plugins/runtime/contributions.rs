use super::*;

pub(super) fn declared_contributions(manifest: &PluginManifest) -> Vec<String> {
    let mut contributions = Vec::new();
    if manifest.workspace_create_policy {
        contributions.push("workspace_create_policy".to_owned());
    }
    if manifest.configuration.is_some() {
        contributions.push("configuration".to_owned());
    }
    if !manifest.ui_surfaces.is_empty() {
        contributions.push("ui_surfaces".to_owned());
    }
    if !manifest.api_routes.is_empty() {
        contributions.push("api_routes".to_owned());
    }
    if !manifest.api_middleware.is_empty() {
        contributions.push("api_middleware".to_owned());
    }
    contributions
}
