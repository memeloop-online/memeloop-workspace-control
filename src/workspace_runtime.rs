use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::config::InstallationId;

pub const WORKSPACE_NAMESPACE: &str = "memeloop-workspace-control";

/// Immutable runtime placement captured when a workspace is created.
///
/// Object names and public routes deliberately are not persisted here. They
/// are a pure function of the installation and workspace short id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, Default)]
pub struct WorkspaceRuntimeIdentity;

impl WorkspaceRuntimeIdentity {
    pub fn new(
        workspace_id: Uuid,
        workspace_short_id: &str,
    ) -> Result<Self, WorkspaceRuntimeIdentityError> {
        if workspace_short_id != workspace_short_id_for(workspace_id) {
            return Err(WorkspaceRuntimeIdentityError::InvalidIdentity);
        }
        Ok(Self)
    }

    pub fn validate(&self) -> Result<(), WorkspaceRuntimeIdentityError> {
        valid_dns_label(WORKSPACE_NAMESPACE, 63)
    }

    pub fn validate_for_workspace(
        &self,
        _installation_id: &InstallationId,
        workspace_id: Uuid,
        workspace_short_id: &str,
    ) -> Result<(), WorkspaceRuntimeIdentityError> {
        self.validate()?;
        (workspace_short_id == workspace_short_id_for(workspace_id))
            .then_some(())
            .ok_or(WorkspaceRuntimeIdentityError::InvalidIdentity)
    }

    pub const fn namespace(&self) -> &'static str {
        WORKSPACE_NAMESPACE
    }
}

/// Canonical, non-persisted names for one workspace runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRuntimeNames {
    pub namespace: String,
    pub resource_prefix: String,
    pub route_key: String,
    pub resources: WorkspaceResourceNames,
}

impl WorkspaceRuntimeNames {
    pub fn for_workspace(
        installation_id: &InstallationId,
        runtime: &WorkspaceRuntimeIdentity,
        workspace_short_id: &str,
    ) -> Result<Self, WorkspaceRuntimeIdentityError> {
        runtime.validate()?;
        valid_dns_label(workspace_short_id, 40)?;
        let resource_prefix = format!("w-{workspace_short_id}");
        let route_key = format!("{}-{workspace_short_id}", installation_id.as_str());
        valid_dns_label(&resource_prefix, 40)?;
        valid_dns_label(&route_key, 63)?;
        let resources = WorkspaceResourceNames::for_prefix(&resource_prefix);
        resources.validate()?;
        Ok(Self {
            namespace: runtime.namespace().to_owned(),
            resource_prefix,
            route_key,
            resources,
        })
    }

    pub fn web_shell_path(&self) -> String {
        format!("/shell/{}/", self.route_key)
    }

    pub fn cluster_admin_binding_name(&self, installation_id: &InstallationId) -> String {
        format!(
            "mwc-{}-{}-admin",
            installation_id.as_str(),
            self.resource_prefix
        )
    }
}

/// Stable, collision-resistant suffix used by API aliases and Kubernetes names.
///
/// UUIDv7's leading bits encode time and are often shared by workspaces created
/// in one batch, so runtime names use the lower 64 random bits instead.
pub fn workspace_short_id_for(workspace_id: Uuid) -> String {
    let encoded_id = workspace_id.simple().to_string();
    encoded_id[encoded_id.len() - 16..].to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceResourceNames {
    pub stateful_set: String,
    pub service: String,
    pub ssh_service: String,
    pub service_account: String,
    pub workspace_config: String,
    pub http_proxy_config: String,
    pub ssh_identity_secret: String,
    pub environment_secret: String,
    pub environment_config_map: String,
    pub files_secret: String,
    pub files_config_map: String,
    pub network_policy: String,
    pub web_shell_ingress: String,
    pub web_shell_envoy_filter: String,
    pub data_claim_template: String,
}

impl WorkspaceResourceNames {
    pub fn for_prefix(prefix: &str) -> Self {
        let named = |suffix: &str| format!("{prefix}-{suffix}");
        Self {
            stateful_set: prefix.to_owned(),
            service: prefix.to_owned(),
            ssh_service: named("ssh"),
            service_account: named("admin"),
            workspace_config: named("config"),
            http_proxy_config: named("http-routes"),
            ssh_identity_secret: named("ssh-identity"),
            environment_secret: named("environment-secret"),
            environment_config_map: named("environment-config"),
            files_secret: named("files-secret"),
            files_config_map: named("files-config"),
            network_policy: named("ingress"),
            web_shell_ingress: named("web-shell"),
            web_shell_envoy_filter: named("ttyd-san"),
            // Keep the claim-template name stable across workspace identity changes. The
            // StatefulSet name is part of the generated PVC name, keeping each workspace claim
            // collision-free in the product Namespace.
            data_claim_template: "workspace-data".to_owned(),
        }
    }

    pub fn pod_ordinal_zero(&self) -> String {
        format!("{}-0", self.stateful_set)
    }

    pub fn data_pvc_ordinal_zero(&self) -> String {
        format!("{}-{}-0", self.data_claim_template, self.stateful_set)
    }

    fn validate(&self) -> Result<(), WorkspaceRuntimeIdentityError> {
        for name in [
            &self.stateful_set,
            &self.service,
            &self.ssh_service,
            &self.service_account,
            &self.workspace_config,
            &self.http_proxy_config,
            &self.ssh_identity_secret,
            &self.environment_secret,
            &self.environment_config_map,
            &self.files_secret,
            &self.files_config_map,
            &self.network_policy,
            &self.web_shell_ingress,
            &self.web_shell_envoy_filter,
            &self.data_claim_template,
            &self.pod_ordinal_zero(),
            &self.data_pvc_ordinal_zero(),
        ] {
            valid_dns_label(name, 63)?;
        }
        Ok(())
    }
}

fn valid_dns_label(value: &str, max_length: usize) -> Result<(), WorkspaceRuntimeIdentityError> {
    let valid_edge = |character: char| character.is_ascii_lowercase() || character.is_ascii_digit();
    if value.is_empty()
        || value.len() > max_length
        || !value
            .chars()
            .all(|character| valid_edge(character) || character == '-')
        || !value.chars().next().is_some_and(valid_edge)
        || !value.chars().last().is_some_and(valid_edge)
    {
        return Err(WorkspaceRuntimeIdentityError::InvalidIdentity);
    }
    Ok(())
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkspaceRuntimeIdentityError {
    #[error("workspace runtime identity is invalid")]
    InvalidIdentity,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_id() -> Uuid {
        Uuid::parse_str("018f0000-0000-7000-8000-000000000001").unwrap()
    }

    #[test]
    fn canonical_names_are_prefixed_and_bound_to_the_installation() {
        let installation = "internal-a".parse().unwrap();
        let short_id = workspace_short_id_for(workspace_id());
        let runtime = WorkspaceRuntimeIdentity::new(workspace_id(), &short_id).unwrap();
        let names =
            WorkspaceRuntimeNames::for_workspace(&installation, &runtime, &short_id).unwrap();
        assert_eq!(names.namespace, WORKSPACE_NAMESPACE);
        assert_eq!(names.resources.stateful_set, "w-8000000000000001");
        assert_eq!(names.resources.data_claim_template, "workspace-data");
        assert_eq!(
            names.resources.data_pvc_ordinal_zero(),
            "workspace-data-w-8000000000000001-0"
        );
        assert_eq!(
            names.web_shell_path(),
            "/shell/internal-a-8000000000000001/"
        );
    }

    #[test]
    fn claim_template_is_stable_while_pvc_names_are_unique_per_workspace() {
        let installation = "internal-a".parse().unwrap();
        let first_id = workspace_id();
        let second_id = Uuid::parse_str("018f0000-0000-7000-8000-000000000002").unwrap();
        let first_short_id = workspace_short_id_for(first_id);
        let second_short_id = workspace_short_id_for(second_id);
        let first_runtime = WorkspaceRuntimeIdentity::new(first_id, &first_short_id).unwrap();
        let second_runtime = WorkspaceRuntimeIdentity::new(second_id, &second_short_id).unwrap();
        let first_names =
            WorkspaceRuntimeNames::for_workspace(&installation, &first_runtime, &first_short_id)
                .unwrap();
        let second_names =
            WorkspaceRuntimeNames::for_workspace(&installation, &second_runtime, &second_short_id)
                .unwrap();

        assert_eq!(
            first_names.resources.data_claim_template,
            second_names.resources.data_claim_template
        );
        assert_eq!(
            first_names.resources.data_pvc_ordinal_zero(),
            "workspace-data-w-8000000000000001-0"
        );
        assert_eq!(
            second_names.resources.data_pvc_ordinal_zero(),
            "workspace-data-w-8000000000000002-0"
        );
        assert_ne!(
            first_names.resources.data_pvc_ordinal_zero(),
            second_names.resources.data_pvc_ordinal_zero()
        );
    }

    #[test]
    fn identity_rejects_a_short_id_from_another_workspace() {
        assert!(WorkspaceRuntimeIdentity::new(workspace_id(), "8000000000000002").is_err());
    }

    #[test]
    fn runtime_identity_always_uses_the_product_namespace() {
        let installation = "internal-a".parse().unwrap();
        let short_id = workspace_short_id_for(workspace_id());
        let runtime = WorkspaceRuntimeIdentity::new(workspace_id(), &short_id).unwrap();
        let names =
            WorkspaceRuntimeNames::for_workspace(&installation, &runtime, &short_id).unwrap();
        assert_eq!(runtime.namespace(), WORKSPACE_NAMESPACE);
        assert_eq!(names.namespace, WORKSPACE_NAMESPACE);
        assert_eq!(names.resource_prefix, "w-8000000000000001");
    }
}
