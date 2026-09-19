use std::sync::Arc;

use axum::Router;
use sha2::{Digest, Sha256};

use crate::{
    config::AppConfig, crypto::EnvelopeCipher, observability::Observability,
    plugins::PluginRuntime, storage::Database,
};

mod admin;
mod auth;
mod catalog;
mod diagnostics;
mod error;
mod events;
mod idempotency;
mod injections;
mod metrics;
mod node_pools;
mod openapi;
mod organization_usage;
mod organizations;
mod plugins;
mod port_mappings;
mod routes;
mod runtime;
mod ssh;
mod system;
mod ui;
mod user_quota;
mod web_shell;
mod webhooks;
mod workspace_creation;
mod workspace_image_update;
mod workspace_placement;
mod workspace_response;
mod workspaces;

pub use error::{ApiError, ErrorBody, ErrorEnvelope};
use openapi::openapi;
use system::{health, ready, system_info};

#[derive(Clone)]
pub struct AppState {
    pub(super) config: AppConfig,
    pub(super) database: Database,
    pub(super) cipher: Option<EnvelopeCipher>,
    internal_auth_token_hash: Option<[u8; 32]>,
    trusted_internal_network: bool,
    pub(super) observability: Observability,
    diagnostics_enabled: bool,
    kubernetes_client: Option<kube::Client>,
    jump_host_public_key: Option<crate::storage::WorkspaceSshPublicIdentity>,
    pub(super) plugins: PluginRuntime,
    organization_metrics: runtime::organization_metrics::OrganizationMetricsCache,
}

impl AppState {
    pub fn new(config: AppConfig, database: Database) -> Self {
        let plugins = PluginRuntime::disabled(database.clone());
        Self {
            config,
            database,
            cipher: None,
            internal_auth_token_hash: None,
            trusted_internal_network: false,
            observability: Observability::default(),
            diagnostics_enabled: false,
            kubernetes_client: None,
            jump_host_public_key: None,
            plugins,
            organization_metrics: Default::default(),
        }
    }

    pub fn with_cipher(config: AppConfig, database: Database, cipher: EnvelopeCipher) -> Self {
        let plugins = PluginRuntime::disabled(database.clone());
        Self {
            config,
            database,
            cipher: Some(cipher),
            internal_auth_token_hash: None,
            trusted_internal_network: false,
            observability: Observability::default(),
            diagnostics_enabled: false,
            kubernetes_client: None,
            jump_host_public_key: None,
            plugins,
            organization_metrics: Default::default(),
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

    pub fn set_plugin_runtime(&mut self, plugins: PluginRuntime) {
        self.plugins = plugins;
    }

    pub fn enable_diagnostics(&mut self) {
        self.diagnostics_enabled = true;
    }

    pub fn observability(&self) -> Observability {
        self.observability.clone()
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
}

pub fn router(state: Arc<AppState>) -> Router {
    routes::router(state)
}

pub fn internal_router(state: Arc<AppState>) -> Router {
    routes::internal_router(state)
}

#[cfg(test)]
mod tests;
