use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;

use crate::{quota::Resources, workspaces::AccessMode};

pub const TEMPLATE_API_VERSION: &str = "workspace.memeloop.dev/v1";
pub const TEMPLATE_KIND: &str = "WorkspaceTemplate";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceTemplateDocument {
    pub api_version: String,
    pub kind: String,
    pub metadata: WorkspaceTemplateMetadata,
    pub spec: WorkspaceTemplateSpec,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceTemplateMetadata {
    pub name: String,
}

/// The complete, immutable-at-workspace-creation template snapshot.
///
/// Everything needed to render a workspace Pod is declared by the selected template and copied
/// into the workspace record. There is no second discriminator beside the template snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceTemplateSpec {
    pub image: String,
    pub access_mode: AccessMode,
    pub resources: Resources,
    pub pod_requests: PodResourceRequest,
    #[serde(default)]
    pub ephemeral_storage_limit_mib: Option<u64>,
    pub workspace_user: String,
    pub workspace_home: String,
    #[serde(default, alias = "preserve_home_root")]
    pub preserve_home_ownership: bool,
    #[serde(default)]
    pub buildkit: bool,
    #[serde(default)]
    pub storage_policy: WorkspaceStoragePolicy,
    #[serde(default)]
    pub cluster_access: bool,
    #[serde(default)]
    pub required_node_names: Vec<String>,
    #[serde(default)]
    pub preferred_node_names: Vec<String>,
    #[serde(default)]
    pub node_selector: BTreeMap<String, String>,
    #[serde(default)]
    #[schema(ignore)]
    pub environment: BTreeMap<String, String>,
}

/// Bounded, Pod-lifetime storage for data that can be regenerated safely.
///
/// The workspace Home PVC remains the durable boundary. These limits do not reserve node disk;
/// Kubernetes enforces them only as upper bounds for the corresponding `emptyDir` volumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WorkspaceStoragePolicy {
    pub runtime_tmp_memory_mib: u64,
    pub build_scratch_gib: u64,
    pub buildkit_cache_gib: u64,
    pub codex_scratch_gib: u64,
    pub home_reserve_mib: Option<u64>,
}

impl Default for WorkspaceStoragePolicy {
    fn default() -> Self {
        Self {
            runtime_tmp_memory_mib: 512,
            build_scratch_gib: 12,
            buildkit_cache_gib: 8,
            codex_scratch_gib: 2,
            home_reserve_mib: None,
        }
    }
}

impl WorkspaceStoragePolicy {
    fn validate(self, disk_gib: u64) -> Result<(), TemplateError> {
        if !(64..=4_096).contains(&self.runtime_tmp_memory_mib)
            || !(1..=256).contains(&self.build_scratch_gib)
            || !(1..=256).contains(&self.buildkit_cache_gib)
            || !(1..=32).contains(&self.codex_scratch_gib)
            || self.home_reserve_mib.is_some_and(|reserve| {
                !(64..=4_096).contains(&reserve)
                    || reserve >= disk_gib.saturating_mul(1_024)
                    || reserve.saturating_mul(10) > disk_gib.saturating_mul(1_024)
            })
        {
            return Err(TemplateError::StoragePolicy);
        }
        Ok(())
    }

    pub(crate) fn effective_home_reserve_mib(self, disk_gib: u64) -> u64 {
        self.home_reserve_mib
            .unwrap_or_else(|| disk_gib.saturating_mul(1_024).saturating_div(10).min(1_024))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PodResourceRequest {
    pub cpu_millis: u64,
    pub memory_mib: u64,
    #[serde(default)]
    pub ephemeral_storage_mib: Option<u64>,
}

impl WorkspaceTemplateDocument {
    pub fn parse(yaml: &str) -> Result<Self, TemplateError> {
        let document: Self = serde_yaml_ng::from_str(yaml).map_err(|_| TemplateError::Yaml)?;
        document.validate()?;
        Ok(document)
    }

    pub fn new(name: impl Into<String>, spec: WorkspaceTemplateSpec) -> Self {
        Self {
            api_version: TEMPLATE_API_VERSION.to_owned(),
            kind: TEMPLATE_KIND.to_owned(),
            metadata: WorkspaceTemplateMetadata { name: name.into() },
            spec,
        }
    }

    pub fn to_yaml(&self) -> Result<String, TemplateError> {
        self.validate()?;
        serde_yaml_ng::to_string(self).map_err(|_| TemplateError::Yaml)
    }

    pub fn validate(&self) -> Result<(), TemplateError> {
        if self.api_version != TEMPLATE_API_VERSION || self.kind != TEMPLATE_KIND {
            return Err(TemplateError::Header);
        }
        let name = self.metadata.name.trim();
        if name.is_empty()
            || name.len() > 120
            || name != self.metadata.name
            || name.chars().any(char::is_control)
        {
            return Err(TemplateError::Name);
        }
        self.spec.validate()
    }

    /// Validates a template submitted through the current authoring API.
    ///
    /// Historical templates and workspace snapshots may still contain `environment`, so regular
    /// parsing remains backward compatible. New mutable environment data belongs to the encrypted,
    /// selector-aware injection cascade instead.
    pub fn validate_authoring(&self) -> Result<(), TemplateError> {
        self.validate()?;
        if !self.spec.environment.is_empty() {
            return Err(TemplateError::ReadOnlyEnvironment);
        }
        Ok(())
    }
}

impl WorkspaceTemplateSpec {
    pub fn validate(&self) -> Result<(), TemplateError> {
        if self.image.trim().is_empty()
            || self.image != self.image.trim()
            || self.image.len() > 512
            || self.image.chars().any(char::is_whitespace)
        {
            return Err(TemplateError::Image);
        }
        if !self.resources.valid_workspace_request() {
            return Err(TemplateError::Resources);
        }
        if self.pod_requests.cpu_millis == 0
            || self.pod_requests.memory_mib == 0
            || self.pod_requests.cpu_millis > self.resources.cpu_millis
            || self.pod_requests.memory_mib > self.resources.memory_mib
            || self
                .pod_requests
                .ephemeral_storage_mib
                .zip(self.ephemeral_storage_limit_mib)
                .is_some_and(|(request, limit)| request > limit)
        {
            return Err(TemplateError::PodResources);
        }
        if !valid_workspace_user(&self.workspace_user)
            || !valid_workspace_home(&self.workspace_home)
        {
            return Err(TemplateError::WorkspaceIdentity);
        }
        if self
            .required_node_names
            .iter()
            .chain(&self.preferred_node_names)
            .any(|name| !valid_selector_part(name))
            || self
                .node_selector
                .iter()
                .any(|(key, value)| !valid_selector_part(key) || !valid_selector_part(value))
        {
            return Err(TemplateError::Scheduling);
        }
        if self
            .environment
            .iter()
            .any(|(key, value)| !valid_env_name(key) || !valid_environment_value(value))
        {
            return Err(TemplateError::Environment);
        }
        self.storage_policy.validate(self.resources.disk_gib)?;
        Ok(())
    }

    pub fn standard(
        image: impl Into<String>,
        access_mode: AccessMode,
        resources: Resources,
    ) -> Self {
        Self {
            image: image.into(),
            access_mode,
            resources,
            pod_requests: PodResourceRequest {
                cpu_millis: resources.cpu_millis,
                memory_mib: resources.memory_mib,
                ephemeral_storage_mib: Some(2_048),
            },
            ephemeral_storage_limit_mib: Some(14_592),
            workspace_user: "workspace".to_owned(),
            workspace_home: "/workspace".to_owned(),
            preserve_home_ownership: false,
            buildkit: false,
            storage_policy: WorkspaceStoragePolicy::default(),
            cluster_access: false,
            required_node_names: Vec::new(),
            preferred_node_names: Vec::new(),
            node_selector: BTreeMap::new(),
            environment: BTreeMap::new(),
        }
    }
}

fn valid_env_name(value: &str) -> bool {
    let mut characters = value.chars();
    matches!(characters.next(), Some(first) if first == '_' || first.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn valid_workspace_user(value: &str) -> bool {
    let mut characters = value.chars();
    value.len() <= 32
        && matches!(characters.next(), Some(first) if first == '_' || first.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '_' | '-')
        })
}

fn valid_workspace_home(value: &str) -> bool {
    value.starts_with('/')
        && value.len() <= 256
        && value != "/"
        && !value.ends_with('/')
        && !value.chars().any(char::is_whitespace)
        && value
            .split('/')
            .skip(1)
            .all(|part| !part.is_empty() && !matches!(part, "." | ".."))
}

fn valid_selector_part(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && !value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
}

fn valid_environment_value(value: &str) -> bool {
    value.len() <= 4_096 && !value.chars().any(char::is_control)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TemplateError {
    #[error("template YAML is invalid")]
    Yaml,
    #[error("template apiVersion or kind is unsupported")]
    Header,
    #[error("template name is invalid")]
    Name,
    #[error("template image reference is invalid")]
    Image,
    #[error("workspace resource limits are invalid")]
    Resources,
    #[error("Pod requests exceed workspace resource limits")]
    PodResources,
    #[error("workspace user or home path is invalid")]
    WorkspaceIdentity,
    #[error("template scheduling constraints are invalid")]
    Scheduling,
    #[error("template environment is invalid")]
    Environment,
    #[error("template environment is read-only compatibility data; use injection items")]
    ReadOnlyEnvironment,
    #[error("template storage policy is invalid")]
    StoragePolicy,
}

#[cfg(test)]
mod tests;
