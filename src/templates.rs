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
    pub workspace_user: String,
    pub workspace_home: String,
    #[serde(default)]
    pub buildkit: bool,
    #[serde(default)]
    pub storage_policy: WorkspaceStoragePolicy,
    #[serde(default)]
    pub cluster_access: bool,
    #[serde(default)]
    pub egress_policy: EgressPolicy,
    /// An optional Kubernetes RuntimeClass for the workspace Pod.
    ///
    /// When omitted, Kubernetes uses its normal runtime selection. A configured class is passed
    /// through verbatim to the PodSpec; Kubernetes rejects the workload if that class is absent.
    #[serde(default)]
    pub runtime_class_name: Option<String>,
    #[serde(default)]
    pub placement: WorkspacePlacement,
    /// Optional browser-accessible desktop endpoint exposed through the
    /// authenticated workspace HTTP gateway. This is a container port, never
    /// a host port, NodePort, or direct RDP endpoint.
    #[serde(default)]
    pub desktop: Option<DesktopEndpoint>,
}

/// The HTTP endpoint served by an image-provided browser desktop (for example,
/// a noVNC or KasmVNC gateway). The workspace controller owns the matching
/// port mapping for the lifetime of the workspace snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DesktopEndpoint {
    pub internal_port: u16,
    #[serde(default)]
    pub display_name: Option<String>,
}

impl DesktopEndpoint {
    fn validate(&self) -> Result<(), TemplateError> {
        if !(1024..=65535).contains(&self.internal_port)
            || is_platform_reserved_port(self.internal_port)
        {
            return Err(TemplateError::Desktop);
        }
        if self.display_name.as_deref().is_some_and(|value| {
            let trimmed = value.trim();
            trimmed.is_empty()
                || trimmed != value
                || trimmed.len() > 80
                || trimmed.chars().any(char::is_control)
        }) {
            return Err(TemplateError::Desktop);
        }
        Ok(())
    }
}

/// Ports reserved for platform listeners. They must not be published as a
/// workspace application endpoint, including a template-owned desktop.
pub fn is_platform_reserved_port(port: u16) -> bool {
    matches!(port, 22 | 2222 | 7681 | 8080 | 8081 | 8443 | 3389)
}

/// The egress boundary applied to a workspace Pod by its NetworkPolicy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EgressPolicy {
    #[default]
    Unrestricted,
    InternetOnly,
}

/// Bounded, Pod-lifetime storage for data that can be regenerated safely.
///
/// The workspace Home PVC remains the durable boundary. Disk-backed sizes become PVC requests
/// when a platform scratch StorageClass is configured, or `emptyDir` bounds otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WorkspaceStoragePolicy {
    /// Total high-speed, Pod-lifetime storage reserved for regenerable workspace data.
    pub temporary_storage_gib: u64,
}

impl Default for WorkspaceStoragePolicy {
    fn default() -> Self {
        Self {
            temporary_storage_gib: 22,
        }
    }
}

impl WorkspaceStoragePolicy {
    fn validate(self) -> Result<(), TemplateError> {
        if !(1..=2_048).contains(&self.temporary_storage_gib) {
            return Err(TemplateError::StoragePolicy);
        }
        Ok(())
    }
}

/// Tenant-visible placement policy. Kubernetes selectors remain private to the selected node pool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WorkspacePlacement {
    pub allowed_node_pools: Vec<String>,
    pub default_node_pool: String,
}

impl Default for WorkspacePlacement {
    fn default() -> Self {
        Self {
            allowed_node_pools: vec!["default".to_owned()],
            default_node_pool: "default".to_owned(),
        }
    }
}

impl WorkspacePlacement {
    fn validate(&self) -> Result<(), TemplateError> {
        if self.allowed_node_pools.is_empty()
            || self.allowed_node_pools.len() > 32
            || !valid_node_pool_name(&self.default_node_pool)
            || !self
                .allowed_node_pools
                .iter()
                .any(|pool| pool == &self.default_node_pool)
        {
            return Err(TemplateError::Placement);
        }
        let mut pools = self.allowed_node_pools.clone();
        if pools.iter().any(|pool| !valid_node_pool_name(pool)) {
            return Err(TemplateError::Placement);
        }
        pools.sort_unstable();
        pools.dedup();
        if pools.len() != self.allowed_node_pools.len() {
            return Err(TemplateError::Placement);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PodResourceRequest {
    pub cpu_millis: u64,
    pub memory_mib: u64,
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

    pub fn validate_authoring(&self) -> Result<(), TemplateError> {
        self.validate()
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
        {
            return Err(TemplateError::PodResources);
        }
        if !valid_workspace_user(&self.workspace_user)
            || !valid_workspace_home(&self.workspace_home)
        {
            return Err(TemplateError::WorkspaceIdentity);
        }
        if self
            .runtime_class_name
            .as_deref()
            .is_some_and(|name| !valid_runtime_class_name(name))
        {
            return Err(TemplateError::RuntimeClass);
        }
        self.placement.validate()?;
        if let Some(desktop) = &self.desktop {
            desktop.validate()?;
        }
        self.storage_policy.validate()?;
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
            },
            workspace_user: "workspace".to_owned(),
            workspace_home: "/workspace".to_owned(),
            buildkit: false,
            storage_policy: WorkspaceStoragePolicy::default(),
            cluster_access: false,
            egress_policy: EgressPolicy::Unrestricted,
            runtime_class_name: None,
            placement: WorkspacePlacement::default(),
            desktop: None,
        }
    }
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

fn valid_node_pool_name(value: &str) -> bool {
    value.len() <= 63
        && !value.is_empty()
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value.bytes().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == b'-'
        })
}

fn valid_runtime_class_name(value: &str) -> bool {
    value.len() <= 63
        && !value.is_empty()
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value.bytes().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == b'-'
        })
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
    #[error("Kubernetes RuntimeClass name is invalid")]
    RuntimeClass,
    #[error("template placement policy is invalid")]
    Placement,
    #[error("template storage policy is invalid")]
    StoragePolicy,
    #[error("template browser desktop endpoint is invalid")]
    Desktop,
}

#[cfg(test)]
mod tests;
