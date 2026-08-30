use std::time::Duration;

use wasmtime::component::Linker;
use wasmtime::{Engine, Store, StoreLimitsBuilder};

use super::{
    LoadedPlugin, PluginApiResponse, PluginRequestContext, WorkspaceCreateContext,
    WorkspaceCreatePlan,
};
use crate::plugins::{Plugin, PluginError, memeloop::workspace_control::types};

const FUEL_LIMIT: u64 = 1_000_000;
const MEMORY_LIMIT: usize = 16 * 1024 * 1024;
const EXECUTION_TIMEOUT: Duration = Duration::from_millis(300);
const EPOCH_TICK: Duration = Duration::from_millis(10);

pub(super) fn invoke(
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
    let mut store = limited_store(engine)?;
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
        .memeloop_workspace_control_plugin_backend()
        .call_admit_create(&mut store, &context, plan)
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

pub(super) fn invoke_api(
    engine: &Engine,
    plugin: &LoadedPlugin,
    route_id: &str,
    context: &PluginRequestContext,
    configuration: &serde_json::Value,
    body: Vec<u8>,
) -> Result<PluginApiResponse, PluginError> {
    const MAX_RESPONSE_BODY: usize = 1024 * 1024;
    let component = plugin
        .component
        .as_ref()
        .ok_or(PluginError::ExecutionFailed)?;
    let mut store = limited_store(engine)?;
    let linker = Linker::new(engine);
    let bindings = Plugin::instantiate(&mut store, component, &linker)
        .map_err(|_| PluginError::ExecutionFailed)?;
    let request = types::ApiRequest {
        context: wit_request_context(context, configuration)?,
        route_id: route_id.to_owned(),
        body,
    };
    let response = bindings
        .memeloop_workspace_control_plugin_backend()
        .call_handle_api(&mut store, &request)
        .map_err(|_| PluginError::ExecutionFailed)?
        .map_err(|_| PluginError::ExecutionFailed)?;
    if !(200..=599).contains(&response.status)
        || response.body.len() > MAX_RESPONSE_BODY
        || !matches!(
            response.content_type.as_str(),
            "application/json" | "text/plain" | "text/plain; charset=utf-8"
        )
    {
        return Err(PluginError::ExecutionFailed);
    }
    Ok(PluginApiResponse {
        status: response.status,
        content_type: response.content_type,
        body: response.body,
    })
}

pub(super) fn invoke_middleware(
    engine: &Engine,
    plugin: &LoadedPlugin,
    context: &PluginRequestContext,
    configuration: &serde_json::Value,
) -> Result<(), PluginError> {
    let component = plugin
        .component
        .as_ref()
        .ok_or(PluginError::ExecutionFailed)?;
    let mut store = limited_store(engine)?;
    let linker = Linker::new(engine);
    let bindings = Plugin::instantiate(&mut store, component, &linker)
        .map_err(|_| PluginError::ExecutionFailed)?;
    let decision = bindings
        .memeloop_workspace_control_plugin_backend()
        .call_check_request(&mut store, &wit_request_context(context, configuration)?)
        .map_err(|_| PluginError::ExecutionFailed)?
        .map_err(|_| PluginError::ExecutionFailed)?;
    if decision.allow {
        return Ok(());
    }
    let code = decision.code.ok_or(PluginError::ExecutionFailed)?;
    if !plugin.manifest.denial_codes.contains(&code) {
        return Err(PluginError::ExecutionFailed);
    }
    Err(PluginError::MiddlewareDenied)
}

fn limited_store(engine: &Engine) -> Result<Store<wasmtime::StoreLimits>, PluginError> {
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
    Ok(store)
}

fn wit_request_context(
    context: &PluginRequestContext,
    configuration: &serde_json::Value,
) -> Result<types::RequestContext, PluginError> {
    Ok(types::RequestContext {
        installation_id: context.installation_id.clone(),
        actor_user_id: context.actor_user_id.map(|value| value.to_string()),
        organization_id: context.organization_id.map(|value| value.to_string()),
        method: context.method.clone(),
        path: context.path.clone(),
        configuration_json: serde_json::to_string(configuration)
            .map_err(|_| PluginError::ExecutionFailed)?,
    })
}

pub(super) fn wit_workspace_plan(workspace: &WorkspaceCreatePlan) -> types::WorkspacePlan {
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
