use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use utoipa::{OpenApi, ToSchema};

use crate::{
    config::{AppConfig, DatabaseMode, InstallationId},
    crypto::EnvelopeCipher,
    storage::Database,
};

mod admin;
mod auth;
mod catalog;
mod error;
mod events;
mod idempotency;
mod injections;
mod metrics;
mod organizations;
mod runtime;
mod ssh;
mod ui;
mod user_quota;
mod web_shell;
mod webhooks;
mod workspace_creation;
mod workspace_response;
mod workspaces;

pub use error::{ApiError, ErrorBody, ErrorEnvelope};

#[derive(Clone)]
pub struct AppState {
    pub(super) config: AppConfig,
    pub(super) database: Database,
    pub(super) cipher: Option<EnvelopeCipher>,
    internal_auth_token_hash: Option<[u8; 32]>,
    trusted_internal_network: bool,
    request_count: Arc<AtomicU64>,
    kubernetes_client: Option<kube::Client>,
    jump_host_public_key: Option<crate::storage::WorkspaceSshPublicIdentity>,
}

impl AppState {
    pub fn new(config: AppConfig, database: Database) -> Self {
        Self {
            config,
            database,
            cipher: None,
            internal_auth_token_hash: None,
            trusted_internal_network: false,
            request_count: Arc::new(AtomicU64::new(0)),
            kubernetes_client: None,
            jump_host_public_key: None,
        }
    }

    pub fn with_cipher(config: AppConfig, database: Database, cipher: EnvelopeCipher) -> Self {
        Self {
            config,
            database,
            cipher: Some(cipher),
            internal_auth_token_hash: None,
            trusted_internal_network: false,
            request_count: Arc::new(AtomicU64::new(0)),
            kubernetes_client: None,
            jump_host_public_key: None,
        }
    }

    pub fn set_internal_auth_token(&mut self, token: &str) {
        self.internal_auth_token_hash = Some(Sha256::digest(token.as_bytes()).into());
    }

    pub fn trust_internal_network(&mut self) {
        self.trusted_internal_network = true;
    }

    pub fn set_kubernetes_client(&mut self, client: kube::Client) {
        self.kubernetes_client = Some(client);
    }

    pub fn set_jump_host_public_key(&mut self, value: &str) -> Result<(), ssh_key::Error> {
        let key = ssh_key::PublicKey::from_openssh(value)?;
        self.jump_host_public_key = Some(crate::storage::WorkspaceSshPublicIdentity {
            algorithm: "ssh-ed25519",
            public_key: key.to_openssh()?,
            fingerprint: key.fingerprint(ssh_key::HashAlg::Sha256).to_string(),
        });
        Ok(())
    }

    fn verify_internal_auth_token(&self, token: &str) -> bool {
        let Some(expected) = self.internal_auth_token_hash else {
            return false;
        };
        let actual: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        expected
            .iter()
            .zip(actual)
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
    }

    fn internal_caller_allowed(&self, token: Option<&str>) -> bool {
        token.is_some_and(|token| self.verify_internal_auth_token(token))
    }

    fn web_shell_internal_caller_allowed(&self, token: Option<&str>) -> bool {
        self.trusted_internal_network || self.internal_caller_allowed(token)
    }

    fn count_request(&self) {
        self.request_count.fetch_add(1, Ordering::Relaxed);
    }
    fn request_count(&self) -> u64 {
        self.request_count.load(Ordering::Relaxed)
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/readyz", get(ready))
        .route("/metrics", get(metrics::prometheus))
        .route("/api/v1/system/info", get(system_info))
        .route("/api/v1/me", get(auth::me))
        .route("/api/v1/events", get(events::stream))
        .route(
            "/api/v1/injections/{scope}/{scope_id}",
            get(injections::list),
        )
        .route(
            "/api/v1/injections/{scope}/{scope_id}/{key}",
            axum::routing::put(injections::replace),
        )
        .route("/api/v1/injections/preview", post(injections::preview))
        .route(
            "/api/v1/organizations",
            get(organizations::list).post(organizations::create),
        )
        .route(
            "/api/v1/admin/users",
            get(admin::list_users).post(admin::create_user),
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
        .route(
            "/api/v1/webhooks",
            get(webhooks::list).post(webhooks::create),
        )
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
        .route("/api/v1/openapi.json", get(openapi))
        .fallback(ui::asset)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            metrics::count,
        ))
        .with_state(state)
}

pub fn internal_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/api/v1/internal/web-shell/authorize",
            get(web_shell::authorize),
        )
        .route(
            "/api/v1/internal/ssh/authorized-key",
            get(ssh::authorized_key),
        )
        .route("/api/v1/internal/ssh/login-users", get(ssh::login_users))
        .with_state(state)
}

#[derive(Debug, Serialize, ToSchema)]
struct HealthResponse {
    status: &'static str,
}

#[utoipa::path(
    get,
    path = "/healthz",
    responses((status = 200, description = "Process is healthy", body = HealthResponse))
)]
async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn ready(State(state): State<Arc<AppState>>) -> Result<Json<HealthResponse>, StatusCode> {
    state
        .database
        .ping()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(HealthResponse { status: "ok" }))
}

#[derive(Debug, Serialize, ToSchema)]
struct SystemInfoResponse {
    installation_id: InstallationId,
    api_version: &'static str,
    database_mode: DatabaseMode,
}

#[utoipa::path(
    get,
    path = "/api/v1/system/info",
    responses((status = 200, description = "Non-sensitive installation metadata", body = SystemInfoResponse))
)]
async fn system_info(State(state): State<Arc<AppState>>) -> Json<SystemInfoResponse> {
    Json(SystemInfoResponse {
        installation_id: state.config.installation_id.clone(),
        api_version: "v1",
        database_mode: state.database.mode(),
    })
}

#[derive(OpenApi)]
#[openapi(
    paths(
        health,
        system_info,
        auth::me,
        events::stream,
        injections::list,
        injections::replace,
        injections::preview,
        organizations::create,
        organizations::list,
        admin::list_users,
        admin::create_user,
        admin::upsert_membership,
        admin::remove_membership,
        admin::get_quota,
        admin::set_quota,
        user_quota::get,
        user_quota::set,
        admin::audit,
        admin::scaling,
        catalog::list_images,
        catalog::put_image,
        catalog::list_templates,
        catalog::create_template,
        catalog::set_template_enabled,
        workspaces::create,
        workspaces::list,
        workspaces::get,
        runtime::list,
        runtime::get,
        workspaces::action
        ,web_shell::issue,
        web_shell::authorize
        ,webhooks::list
        ,webhooks::create
        ,ssh::authorized_key
        ,ssh::login_users
    ),
    components(schemas(
        HealthResponse,
        SystemInfoResponse,
        InstallationId,
        DatabaseMode,
        crate::storage::Principal,
        crate::storage::Organization,
        crate::storage::UserSummary,
        crate::storage::AuditRecord,
        crate::storage::JobCounts,
        crate::storage::ImagePolicy,
        crate::storage::WorkspaceTemplate,
        crate::storage::WorkspaceSshPublicIdentity,
        crate::storage::CreateWorkspaceTemplate,
        crate::storage::CreateOrganization,
        crate::storage::CreateWorkspace,
        workspaces::CreateWorkspaceRequest,
        crate::workspaces::Workspace,
        crate::workspaces::WorkspaceState,
        crate::workspaces::AccessMode,
        crate::workspaces::WorkspaceRuntimeProfile,
        crate::quota::Resources,
        crate::injections::InjectionItem,
        crate::injections::InjectionValue,
        crate::injections::InjectionKind,
        crate::injections::InjectionScope,
        crate::storage::InjectionScopeRef,
        crate::storage::StoredInjectionSummary,
        crate::storage::EventRecord,
        crate::storage::IssuedWebShellTicket,
        crate::storage::CreateWebhookSubscription,
        crate::storage::WebhookSubscriptionSummary,
        crate::injections::ResolvedInjectionSummary,
        injections::PreviewRequest,
        workspaces::WorkspaceResponse,
        runtime::WorkspaceRuntimeResponse,
        runtime::WorkspaceRuntimeEntry,
        runtime::PodRuntime,
        runtime::PodMetric,
        runtime::PodEvent,
        web_shell::WebShellTicketResponse,
        admin::CreateUserRequest,
        admin::MembershipRequest,
        admin::ScalingResponse,
        catalog::PutImageRequest,
        ErrorEnvelope,
        ErrorBody
    )),
    tags((name = "system", description = "Control plane health and metadata"))
)]
struct ApiDoc;

async fn openapi() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

#[cfg(test)]
mod tests;
