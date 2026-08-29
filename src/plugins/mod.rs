mod error;
mod manifest;
mod runtime;
mod schema;

wasmtime::component::bindgen!({
    world: "plugin",
    path: "wit/workspace-control.wit",
});

pub use error::PluginError;
pub use manifest::{PluginConfigurationContribution, PluginManifest};
pub use runtime::{PluginRuntime, WorkspaceCreateContext, WorkspaceCreatePlan};

use manifest::discover;
use schema::ConfigurationSchema;
