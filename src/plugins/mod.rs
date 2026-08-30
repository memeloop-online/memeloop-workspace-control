mod error;
mod manifest;
mod runtime;
mod schema;

wasmtime::component::bindgen!({
    world: "plugin",
    path: "wit/workspace-control.wit",
});

pub use error::PluginError;
pub(crate) use manifest::validate_plugin_content;
pub use manifest::{
    PluginApiMiddleware, PluginApiRoute, PluginAssetDescriptor, PluginConfigurationContribution,
    PluginManifest, PluginRoutePermission, PluginUiPlacement, PluginUiSurface,
};
pub(crate) use runtime::RuntimePluginView;
pub use runtime::{
    PluginApiResponse, PluginRequestContext, PluginRuntime, PluginRuntimeMetrics,
    WorkspaceCreateContext, WorkspaceCreatePlan,
};

use manifest::discover;
use schema::ConfigurationSchema;
