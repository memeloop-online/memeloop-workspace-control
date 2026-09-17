mod builder;
mod buildkit;
mod client;
#[cfg(test)]
mod client_tests;
pub mod config;
mod egress_refresh;
mod envoy_filter;
mod higress;
mod http_proxy;
mod internet_egress;
mod materialization;
mod network_policy;
mod network_refresh;
pub use network_refresh::NetworkRefreshError;
mod ownership;
mod port_mappings;
mod resource_helpers;
mod ttyd_mtls;
mod workload;
mod workspace_pod;

pub use builder::{BuildError, DesiredResources, ResourceBuilder};
pub use client::{DeleteProgress, KubernetesCoordinator, ReconcileError, workspace_ssh_node_port};
pub use egress_refresh::{
    DynamicEgressRefresh, DynamicEgressRefreshConfig, DynamicEgressRefreshError,
};
pub use internet_egress::{InternetEgressConfig, InternetEgressConfigError};
pub use materialization::{InjectionMaterialization, MaterializationError};
pub use ownership::OwnershipError;
pub use ttyd_mtls::{TtydMtlsConfig, TtydMtlsConfigError};

pub(crate) use resource_helpers::namespaced_metadata;

pub use crate::workspaces::ResolvedPlacement;

pub const OWNER_INSTALLATION_LABEL: &str = "workspace.memeloop.dev/owner-installation";
pub const WORKSPACE_ID_LABEL: &str = "workspace.memeloop.dev/workspace-id";
pub const ORGANIZATION_ID_LABEL: &str = "workspace.memeloop.dev/organization-id";
pub const OWNER_USER_ID_LABEL: &str = "workspace.memeloop.dev/owner-user-id";
pub const STORAGE_ROLE_LABEL: &str = "workspace.memeloop.dev/storage-role";
/// Carries the immutable template identity into Prometheus' `kube_pod_labels`
/// series so a template-restricted API key can never obtain an organization-wide
/// aggregate and filter it after the fact.
pub const TEMPLATE_ID_LABEL: &str = "workspace.memeloop.dev/template-id";
const COMPONENT_LABEL: &str = "app.kubernetes.io/component";
const MANAGED_BY_LABEL: &str = "app.kubernetes.io/managed-by";
