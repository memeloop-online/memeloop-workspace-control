use std::sync::Arc;

use axum::{
    Router,
    routing::{get, post},
};

use super::{
    AppState, admin, auth, catalog, diagnostics, events, health, injections, metrics, openapi,
    organizations, plugins, port_mappings, ready, runtime, ssh, system_info, ui, user_quota,
    web_shell, webhooks, workspaces,
};

pub(super) fn router(state: Arc<AppState>) -> Router {
    let router = system_and_identity_routes(Router::new());
    let router = organization_routes(router);
    let router = plugin_routes(router);
    let router = workspace_routes(router);
    router
        .route("/api/v1/openapi.json", get(openapi))
        .fallback(ui::asset)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            metrics::count,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            plugins::api_middleware,
        ))
        .with_state(state)
}

type ApiRouter = Router<Arc<AppState>>;

fn system_and_identity_routes(router: ApiRouter) -> ApiRouter {
    router
        .route("/livez", get(health))
        .route("/healthz", get(health))
        .route("/readyz", get(ready))
        .route("/metrics", get(metrics::prometheus))
        .route("/api/v1/system/info", get(system_info))
        .route("/api/v1/me", get(auth::me))
        .route(
            "/api/v1/me/profile",
            get(admin::get_profile).put(admin::update_profile),
        )
        .route(
            "/api/v1/me/api-keys",
            get(admin::list_api_keys).post(admin::create_api_key),
        )
        .route(
            "/api/v1/me/api-keys/{key_id}",
            axum::routing::delete(admin::delete_api_key),
        )
        .route("/api/v1/events", get(events::stream))
        .route(
            "/api/v1/injections/{scope}/{scope_id}",
            get(injections::list),
        )
        .route(
            "/api/v1/injections/{scope}/{scope_id}/batch-delete",
            post(injections::batch_delete),
        )
        .route(
            "/api/v1/injections/{scope}/{scope_id}/{key}",
            axum::routing::put(injections::replace).delete(injections::delete),
        )
        .route("/api/v1/injections/preview", post(injections::preview))
}

fn organization_routes(router: ApiRouter) -> ApiRouter {
    router
        .route(
            "/api/v1/organizations",
            get(organizations::list_page).post(organizations::create),
        )
        .route(
            "/api/v1/organizations/{organization_id}",
            axum::routing::put(organizations::update).delete(organizations::delete),
        )
        .route(
            "/api/v1/admin/users",
            get(admin::list_users_page).post(admin::create_user),
        )
        .route(
            "/api/v1/admin/users/{user_id}",
            axum::routing::put(admin::update_user),
        )
        .route(
            "/api/v1/admin/users/{user_id}/api-keys",
            get(admin::list_user_api_keys),
        )
        .route(
            "/api/v1/admin/users/{user_id}/api-keys/{key_id}",
            axum::routing::delete(admin::admin_revoke_api_key),
        )
        .route(
            "/api/v1/organizations/{organization_id}/members",
            get(admin::list_members),
        )
        .route(
            "/api/v1/organizations/{organization_id}/members/{user_id}",
            axum::routing::put(admin::upsert_membership).delete(admin::remove_membership),
        )
        .route(
            "/api/v1/organizations/{organization_id}/quota",
            get(admin::get_quota).put(admin::set_quota),
        )
        .route(
            "/api/v1/admin/users/{user_id}/quota",
            get(user_quota::get).put(user_quota::set),
        )
        .route("/api/v1/audit", get(admin::audit))
        .route("/api/v1/admin/scaling", get(admin::scaling))
}

fn plugin_routes(router: ApiRouter) -> ApiRouter {
    router
        .route("/api/v1/plugins", get(plugins::list_packages))
        .route(
            "/api/v1/plugins/inspections/upload",
            post(plugins::inspect_upload)
                // The validated package payload is capped at 80 MiB; multipart boundaries and
                // per-part headers require a small transport allowance above that payload limit.
                .layer(axum::extract::DefaultBodyLimit::max(82 * 1024 * 1024)),
        )
        .route(
            "/api/v1/plugins/inspections/url",
            post(plugins::inspect_url),
        )
        .route(
            "/api/v1/plugins/inspections/github-release",
            post(plugins::inspect_github_release),
        )
        .route("/api/v1/plugins/installs", post(plugins::confirm_install))
        .route(
            "/api/v1/plugins/{plugin_id}/enabled",
            axum::routing::put(plugins::set_enabled),
        )
        .route(
            "/api/v1/plugins/{plugin_id}",
            axum::routing::delete(plugins::uninstall),
        )
        .route(
            "/api/v1/plugins/{plugin_id}/ui-surfaces/{surface_id}/sessions",
            post(plugins::create_surface_session),
        )
        .route(
            "/api/v1/plugin-ui/{plugin_id}/{session_id}/bridge",
            post(plugins::bridge),
        )
        .route(
            "/api/v1/plugin-ui/{plugin_id}/{session_id}/{*asset_path}",
            get(plugins::surface_asset),
        )
        .route(
            "/api/v1/plugin-api/{plugin_id}/{route_id}/{*path}",
            axum::routing::any(plugins::invoke_api_route)
                .layer(axum::extract::DefaultBodyLimit::max(256 * 1024)),
        )
        .route(
            "/api/v1/plugins/{plugin_id}/configuration",
            get(plugins::get_configuration)
                .put(plugins::put_configuration)
                .delete(plugins::delete_configuration),
        )
        .route(
            "/api/v1/webhooks",
            get(webhooks::list).post(webhooks::create),
        )
}

fn workspace_routes(router: ApiRouter) -> ApiRouter {
    router
        .route(
            "/api/v1/admin/images",
            get(catalog::list_images).put(catalog::put_image),
        )
        .route(
            "/api/v1/templates",
            get(catalog::list_templates).post(catalog::create_template),
        )
        .route(
            "/api/v1/templates/{template_id}",
            axum::routing::put(catalog::replace_template).delete(catalog::delete_template),
        )
        .route(
            "/api/v1/templates/{template_id}/enabled",
            axum::routing::put(catalog::set_template_enabled),
        )
        .route(
            "/api/v1/workspaces",
            get(workspaces::list).post(workspaces::create),
        )
        .route("/api/v1/workspaces/{workspace_id}", get(workspaces::get))
        .route("/api/v1/workspace-runtimes", get(runtime::list))
        .route(
            "/api/v1/workspaces/{workspace_id}/runtime",
            get(runtime::get),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/actions/{action}",
            post(workspaces::action),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/web-shell-tickets",
            post(web_shell::issue),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/port-mappings",
            get(port_mappings::list).post(port_mappings::create),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/port-mappings/{mapping_id}",
            axum::routing::delete(port_mappings::delete),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/port-mappings/{mapping_id}/open",
            post(port_mappings::open),
        )
}

pub(super) fn internal_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/livez", get(health))
        .route("/readyz", get(ready))
        .route("/metrics", get(metrics::prometheus))
        .route("/debug/pprof/profile", get(diagnostics::cpu_profile))
        .route("/debug/pprof/heap", get(diagnostics::heap_profile))
        .route("/diagnostics/process", get(diagnostics::process))
        .route(
            "/api/v1/internal/web-shell/authorize",
            get(web_shell::authorize),
        )
        .route(
            "/api/v1/internal/port-mappings/authorize",
            get(port_mappings::authorize),
        )
        .route(
            "/api/v1/internal/ssh/authorized-key",
            get(ssh::authorized_key),
        )
        .route("/api/v1/internal/ssh/login-users", get(ssh::login_users))
        .with_state(state)
}
