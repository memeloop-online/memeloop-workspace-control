use std::{
    path::Path,
    sync::{Arc, RwLock},
    time::Duration,
};

use serde::Serialize;
use tokio::sync::{Mutex, Semaphore};
use uuid::Uuid;
use wasmtime::component::Component;
use wasmtime::{Config, Engine};

use crate::{storage::Database, templates::WorkspaceTemplateSpec};

use super::{ConfigurationSchema, PluginError, PluginManifest, discover};

const EPOCH_TICK: Duration = Duration::from_millis(10);
const MAX_CONCURRENT_EXECUTIONS: usize = 16;

#[derive(Clone)]
struct LoadedPlugin {
    manifest: PluginManifest,
    component: Option<Component>,
    configuration_schema: Option<ConfigurationSchema>,
    source_kind: String,
    source_ref: String,
    package_digest: String,
    source_confirmation: String,
    approved_contributions: Vec<String>,
    package_version: u64,
    enabled: bool,
}

mod execution;
use execution::{invoke, invoke_api, invoke_middleware, wit_workspace_plan};
mod catalog;
mod configuration;

#[derive(Default)]
struct RuntimeRegistry {
    catalog_revision: u64,
    plugins: Vec<LoadedPlugin>,
}

#[derive(Clone)]
pub struct PluginRuntime {
    engine: Option<Engine>,
    plugins: Arc<RwLock<RuntimeRegistry>>,
    database: Database,
    execution_slots: Arc<Semaphore>,
    refresh_lock: Arc<Mutex<()>>,
}

#[derive(Clone, Debug)]
pub(crate) struct RuntimePluginView {
    pub manifest: PluginManifest,
    pub source_kind: String,
    pub source_ref: String,
    pub package_digest: String,
    pub source_confirmation: String,
    pub approved_contributions: Vec<String>,
    pub package_version: u64,
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct WorkspaceCreateContext {
    pub installation_id: String,
    pub actor_user_id: Uuid,
    pub organization_id: Uuid,
    pub owner_id: Uuid,
    pub template_id: Uuid,
}

#[derive(Clone, Debug, Serialize)]
pub struct WorkspaceCreatePlan {
    pub name: String,
    pub image: String,
    pub access_mode: String,
    pub cpu_millis: u64,
    pub memory_mib: u64,
    pub gpu_count: u32,
    pub disk_gib: u64,
    pub buildkit_enabled: bool,
    pub cluster_access: bool,
}

#[derive(Clone, Debug)]
pub struct PluginRequestContext {
    pub installation_id: String,
    pub actor_user_id: Option<Uuid>,
    pub organization_id: Option<Uuid>,
    pub method: String,
    pub path: String,
}

#[derive(Clone, Debug)]
pub struct PluginApiResponse {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct PluginRuntimeMetrics {
    pub loaded: usize,
    pub enabled: usize,
    pub executable: usize,
    pub executions_active: usize,
    pub execution_limit: usize,
    pub registry_metadata_bytes_estimate: usize,
}

impl WorkspaceCreatePlan {
    pub fn from_template(name: &str, template: &WorkspaceTemplateSpec) -> Self {
        Self {
            name: name.trim().to_owned(),
            image: template.image.clone(),
            access_mode: template.access_mode.as_str().to_owned(),
            cpu_millis: template.resources.cpu_millis,
            memory_mib: template.resources.memory_mib,
            gpu_count: template.resources.gpu_count,
            disk_gib: template.resources.disk_gib,
            buildkit_enabled: template.buildkit,
            cluster_access: template.cluster_access,
        }
    }
}

impl PluginRuntime {
    pub fn disabled(database: Database) -> Self {
        Self {
            engine: None,
            plugins: Arc::new(RwLock::new(RuntimeRegistry::default())),
            database,
            execution_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_EXECUTIONS)),
            refresh_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn runtime_metrics(&self) -> PluginRuntimeMetrics {
        let registry = self.plugins.read().expect("plugin runtime lock poisoned");
        PluginRuntimeMetrics {
            loaded: registry.plugins.len(),
            enabled: registry
                .plugins
                .iter()
                .filter(|plugin| plugin.enabled)
                .count(),
            executable: registry
                .plugins
                .iter()
                .filter(|plugin| plugin.component.is_some())
                .count(),
            executions_active: MAX_CONCURRENT_EXECUTIONS
                .saturating_sub(self.execution_slots.available_permits()),
            execution_limit: MAX_CONCURRENT_EXECUTIONS,
            registry_metadata_bytes_estimate: registry
                .plugins
                .iter()
                .map(|plugin| {
                    serde_json::to_vec(&plugin.manifest).map_or(0, |value| value.len())
                        + plugin.source_kind.len()
                        + plugin.source_ref.len()
                        + plugin.package_digest.len()
                        + plugin.source_confirmation.len()
                })
                .sum(),
        }
    }

    pub fn load(root: Option<&Path>, database: Database) -> Result<Self, PluginError> {
        let packages = root.map(discover).transpose()?.unwrap_or_default();
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.consume_fuel(true);
        config.epoch_interruption(true);
        let engine = Engine::new(&config).map_err(|_| PluginError::RuntimeUnavailable)?;
        let mut plugins = Vec::with_capacity(packages.len());
        for package in packages {
            let mounted_name = package.manifest.id.clone();
            let component = package
                .component_path
                .as_ref()
                .map(|path| {
                    Component::from_file(&engine, path)
                        .map_err(|_| PluginError::invalid("component could not be compiled"))
                })
                .transpose()?;
            plugins.push(LoadedPlugin {
                manifest: package.manifest,
                component,
                configuration_schema: package.configuration_schema,
                source_kind: "mounted".to_owned(),
                source_ref: mounted_name,
                package_digest: String::new(),
                source_confirmation: "gitops_mounted".to_owned(),
                approved_contributions: Vec::new(),
                package_version: 0,
                enabled: true,
            });
        }
        for plugin in &mut plugins {
            plugin.approved_contributions = declared_contributions(&plugin.manifest);
        }
        let epoch_engine = engine.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(EPOCH_TICK);
            loop {
                interval.tick().await;
                epoch_engine.increment_epoch();
            }
        });
        Ok(Self {
            engine: Some(engine),
            plugins: Arc::new(RwLock::new(RuntimeRegistry {
                catalog_revision: 0,
                plugins,
            })),
            database,
            execution_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_EXECUTIONS)),
            refresh_lock: Arc::new(Mutex::new(())),
        })
    }

    pub async fn admit_workspace_create(
        &self,
        context: WorkspaceCreateContext,
        plan: WorkspaceCreatePlan,
    ) -> Result<(), PluginError> {
        self.synchronize().await?;
        let policies: Vec<_> = self
            .plugins
            .read()
            .map_err(|_| PluginError::RuntimeUnavailable)?
            .plugins
            .iter()
            .filter(|plugin| {
                plugin.enabled
                    && plugin.manifest.workspace_create_policy
                    && plugin
                        .approved_contributions
                        .iter()
                        .any(|value| value == "workspace_create_policy")
            })
            .cloned()
            .collect();
        if policies.is_empty() {
            return Ok(());
        }
        let engine = self.engine.clone().ok_or(PluginError::RuntimeUnavailable)?;
        let mut configured = Vec::with_capacity(policies.len());
        for plugin in policies {
            let configuration = self
                .effective_configuration(&plugin, Some(context.organization_id))
                .await?;
            configured.push((plugin, configuration));
        }
        let permit = self
            .execution_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| PluginError::RuntimeUnavailable)?;
        let plan = wit_workspace_plan(&plan);
        let call = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            for (plugin, configuration) in configured {
                invoke(&engine, &plugin, &context, &plan, &configuration)?;
            }
            Ok(())
        });
        tokio::time::timeout(Duration::from_millis(500), call)
            .await
            .map_err(|_| PluginError::ExecutionFailed)?
            .map_err(|_| PluginError::ExecutionFailed)?
    }

    pub async fn invoke_api_route(
        &self,
        plugin_id: &str,
        route_id: &str,
        context: PluginRequestContext,
        body: Vec<u8>,
    ) -> Result<PluginApiResponse, PluginError> {
        const MAX_REQUEST_BODY: usize = 256 * 1024;
        if body.len() > MAX_REQUEST_BODY {
            return Err(PluginError::InvalidApiRequest);
        }
        self.synchronize().await?;
        let plugin = self
            .plugins
            .read()
            .map_err(|_| PluginError::RuntimeUnavailable)?
            .plugins
            .iter()
            .find(|plugin| {
                plugin.enabled
                    && plugin.manifest.id == plugin_id
                    && plugin
                        .approved_contributions
                        .iter()
                        .any(|value| value == "api_routes")
                    && plugin
                        .manifest
                        .api_routes
                        .iter()
                        .any(|route| route.id == route_id)
            })
            .cloned()
            .ok_or(PluginError::NotFound)?;
        let configuration = self
            .effective_configuration(&plugin, context.organization_id)
            .await?;
        let engine = self.engine.clone().ok_or(PluginError::RuntimeUnavailable)?;
        let permit = self
            .execution_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| PluginError::RuntimeUnavailable)?;
        let route_id = route_id.to_owned();
        let call = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            invoke_api(&engine, &plugin, &route_id, &context, &configuration, body)
        });
        tokio::time::timeout(Duration::from_millis(500), call)
            .await
            .map_err(|_| PluginError::ExecutionFailed)?
            .map_err(|_| PluginError::ExecutionFailed)?
    }

    pub async fn check_api_middleware(
        &self,
        context: PluginRequestContext,
    ) -> Result<(), PluginError> {
        let plugins = self
            .plugins
            .read()
            .map_err(|_| PluginError::RuntimeUnavailable)?
            .plugins
            .iter()
            .filter(|plugin| {
                plugin.enabled
                    && !plugin.manifest.api_middleware.is_empty()
                    && plugin
                        .approved_contributions
                        .iter()
                        .any(|value| value == "api_middleware")
            })
            .cloned()
            .collect::<Vec<_>>();
        if plugins.is_empty() {
            return Ok(());
        }
        let mut configured = Vec::with_capacity(plugins.len());
        for plugin in plugins {
            let configuration = self
                .effective_configuration(&plugin, context.organization_id)
                .await?;
            configured.push((plugin, configuration));
        }
        let engine = self.engine.clone().ok_or(PluginError::RuntimeUnavailable)?;
        let permit = self
            .execution_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| PluginError::RuntimeUnavailable)?;
        let call = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            for (plugin, configuration) in configured {
                invoke_middleware(&engine, &plugin, &context, &configuration)?;
            }
            Ok(())
        });
        tokio::time::timeout(Duration::from_millis(500), call)
            .await
            .map_err(|_| PluginError::ExecutionFailed)?
            .map_err(|_| PluginError::ExecutionFailed)?
    }
}

fn declared_contributions(manifest: &PluginManifest) -> Vec<String> {
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
