use std::{path::Path, sync::Arc, time::Duration};

use serde::Serialize;
use tokio::sync::Semaphore;
use uuid::Uuid;
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store, StoreLimitsBuilder};

use crate::{storage::Database, templates::WorkspaceTemplateSpec};

use super::{
    ConfigurationSchema, Plugin, PluginError, PluginManifest, discover,
    memeloop::workspace_control::types,
};

const FUEL_LIMIT: u64 = 1_000_000;
const MEMORY_LIMIT: usize = 16 * 1024 * 1024;
const EXECUTION_TIMEOUT: Duration = Duration::from_millis(300);
const EPOCH_TICK: Duration = Duration::from_millis(10);
const MAX_CONCURRENT_EXECUTIONS: usize = 16;

#[derive(Clone)]
struct LoadedPlugin {
    manifest: PluginManifest,
    component: Option<Component>,
    configuration_schema: Option<ConfigurationSchema>,
}

#[derive(Clone)]
pub struct PluginRuntime {
    engine: Option<Engine>,
    plugins: Arc<Vec<LoadedPlugin>>,
    database: Database,
    execution_slots: Arc<Semaphore>,
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
            plugins: Arc::default(),
            database,
            execution_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_EXECUTIONS)),
        }
    }

    pub fn load(root: Option<&Path>, database: Database) -> Result<Self, PluginError> {
        let Some(root) = root else {
            return Ok(Self::disabled(database));
        };
        let packages = discover(root)?;
        if packages.is_empty() {
            return Ok(Self::disabled(database));
        }
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.consume_fuel(true);
        config.epoch_interruption(true);
        let engine = Engine::new(&config).map_err(|_| PluginError::RuntimeUnavailable)?;
        let mut plugins = Vec::with_capacity(packages.len());
        for package in packages {
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
            });
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
            plugins: Arc::new(plugins),
            database,
            execution_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_EXECUTIONS)),
        })
    }

    pub fn manifests(&self) -> Vec<PluginManifest> {
        self.plugins
            .iter()
            .map(|plugin| plugin.manifest.clone())
            .collect()
    }

    pub fn manifest(&self, plugin_id: &str) -> Option<PluginManifest> {
        self.plugins
            .iter()
            .find(|plugin| plugin.manifest.id == plugin_id)
            .map(|plugin| plugin.manifest.clone())
    }

    pub fn validate_configuration(
        &self,
        plugin_id: &str,
        value: &serde_json::Value,
    ) -> Result<(), PluginError> {
        let plugin = self
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
        let plugin = self
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

    pub async fn admit_workspace_create(
        &self,
        context: WorkspaceCreateContext,
        plan: WorkspaceCreatePlan,
    ) -> Result<(), PluginError> {
        let policies: Vec<_> = self
            .plugins
            .iter()
            .filter(|plugin| plugin.manifest.workspace_create_policy)
            .cloned()
            .collect();
        if policies.is_empty() {
            return Ok(());
        }
        let engine = self.engine.clone().ok_or(PluginError::RuntimeUnavailable)?;
        let mut configured = Vec::with_capacity(policies.len());
        for plugin in policies {
            let configuration = self
                .effective_configuration(&plugin, context.organization_id)
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

    async fn effective_configuration(
        &self,
        plugin: &LoadedPlugin,
        organization_id: Uuid,
    ) -> Result<serde_json::Value, PluginError> {
        let configured = self
            .database
            .plugin_configuration_for_scope(&plugin.manifest.id, Some(organization_id))
            .await?
            .or(self
                .database
                .plugin_configuration_for_scope(&plugin.manifest.id, None)
                .await?);
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

fn invoke(
    engine: &Engine,
    plugin: &LoadedPlugin,
    context: &WorkspaceCreateContext,
    plan: &types::WorkspacePlan,
    configuration: &serde_json::Value,
) -> Result<(), PluginError> {
    let component = plugin
        .component
        .as_ref()
        .ok_or(PluginError::ExecutionFailed)?;
    let limits = StoreLimitsBuilder::new()
        .memory_size(MEMORY_LIMIT)
        .instances(8)
        .tables(2)
        .memories(2)
        .build();
    let mut store = Store::new(engine, limits);
    store.limiter(|limits| limits);
    store.set_epoch_deadline(
        u64::try_from(
            EXECUTION_TIMEOUT
                .as_millis()
                .div_ceil(EPOCH_TICK.as_millis()),
        )
        .unwrap_or(u64::MAX)
        .max(1),
    );
    store
        .set_fuel(FUEL_LIMIT)
        .map_err(|_| PluginError::ExecutionFailed)?;
    let linker = Linker::new(engine);
    let bindings = Plugin::instantiate(&mut store, component, &linker)
        .map_err(|_| PluginError::ExecutionFailed)?;
    let context = types::CreateContext {
        installation_id: context.installation_id.clone(),
        actor_user_id: context.actor_user_id.to_string(),
        organization_id: context.organization_id.to_string(),
        owner_id: context.owner_id.to_string(),
        template_id: context.template_id.to_string(),
        configuration_json: serde_json::to_string(configuration)
            .map_err(|_| PluginError::ExecutionFailed)?,
    };
    let decision = bindings
        .memeloop_workspace_control_workspace_create_policy()
        .call_evaluate(&mut store, &context, plan)
        .map_err(|_| PluginError::ExecutionFailed)?
        .map_err(|_| PluginError::ExecutionFailed)?;
    if decision.allow {
        return Ok(());
    }
    let code = decision.code.ok_or(PluginError::ExecutionFailed)?;
    if !plugin.manifest.denial_codes.contains(&code) {
        return Err(PluginError::ExecutionFailed);
    }
    Err(PluginError::AdmissionDenied {
        plugin_id: plugin.manifest.id.clone(),
        decision_code: code,
    })
}

fn wit_workspace_plan(workspace: &WorkspaceCreatePlan) -> types::WorkspacePlan {
    types::WorkspacePlan {
        name: workspace.name.clone(),
        image: workspace.image.clone(),
        access_mode: workspace.access_mode.clone(),
        cpu_millis: workspace.cpu_millis,
        memory_mib: workspace.memory_mib,
        gpu_count: workspace.gpu_count,
        disk_gib: workspace.disk_gib,
        buildkit_enabled: workspace.buildkit_enabled,
        cluster_access: workspace.cluster_access,
    }
}
