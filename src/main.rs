use std::{collections::BTreeMap, io, sync::Arc};

use clap::Parser;
use memeloop_workspace_control::{
    admin::{Cli, Command, execute_admin, execute_database},
    api::{AppState, internal_router, router},
    config::AppConfig,
    crypto::EnvelopeCipher,
    jobs::{ControlPlaneJobHandler, JobWorker, WebhookDeliveryHandler, WorkspaceReconcileHandler},
    kubernetes::{KubernetesCoordinator, ResourceBuilder},
    plugins::PluginRuntime,
    storage::Database,
};
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let cli = Cli::parse();
    let config = cli.config;
    config.validate()?;
    let database = Database::connect(&config.database_url, config.installation_id.clone()).await?;
    database.migrate().await?;
    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => serve(config, database).await?,
        Command::Admin { command } => execute_admin(&database, command).await?,
        Command::Database { command } => execute_database(&database, command).await?,
    }
    Ok(())
}

async fn serve(config: AppConfig, database: Database) -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(config.listen_address).await?;
    let address = listener.local_addr()?;
    let installation_id = config.installation_id.clone();
    let cipher = load_cipher()?;
    let plugins = PluginRuntime::load(config.plugin_dir.as_deref(), database.clone())?;
    plugins.synchronize().await?;
    plugins.spawn_catalog_refresher();
    let kubernetes_enabled = env_bool("MWC_KUBERNETES_ENABLED", false)?;
    let diagnostics_enabled = env_bool("MWC_DIAGNOSTICS_ENABLED", false)?;
    #[cfg(target_os = "linux")]
    if diagnostics_enabled {
        jemalloc_pprof::activate_jemalloc_profiling().await;
    }
    let (kubernetes_client, workspace_handler) =
        kubernetes_runtime(&config, &database, &cipher, kubernetes_enabled).await?;
    let state = app_state(
        &config,
        database.clone(),
        cipher.clone(),
        kubernetes_client,
        kubernetes_enabled,
        diagnostics_enabled,
        plugins,
    )?;
    let worker = job_worker(
        &config,
        &database,
        &cipher,
        workspace_handler,
        state.observability(),
    )?;
    let internal_listen_address =
        internal_listener_address(kubernetes_enabled, diagnostics_enabled)?;
    let (shutdown_tx, _) = tokio::sync::watch::channel(false);
    let worker_handle = worker.map(|worker| {
        let shutdown = shutdown_tx.subscribe();
        tokio::spawn(async move { worker.run_until_shutdown(shutdown).await })
    });
    let internal_handle = start_internal_listener(
        internal_listen_address,
        state.clone(),
        shutdown_tx.subscribe(),
    )
    .await?;
    let app = router(Arc::new(state));
    let signal_tx = shutdown_tx.clone();

    info!(%address, %installation_id, kubernetes_enabled, "control plane listening");
    let server_result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            let _ = signal_tx.send(true);
        })
        .await;
    let _ = shutdown_tx.send(true);
    await_background_tasks(worker_handle, internal_handle).await?;
    server_result?;
    Ok(())
}

type InternalServerHandle = tokio::task::JoinHandle<Result<(), io::Error>>;
type WorkerHandle =
    tokio::task::JoinHandle<Result<(), memeloop_workspace_control::jobs::JobWorkerError>>;

async fn start_internal_listener(
    address: Option<std::net::SocketAddr>,
    mut state: AppState,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<Option<InternalServerHandle>, io::Error> {
    let Some(address) = address else {
        return Ok(None);
    };
    state.trust_internal_network();
    let listener = TcpListener::bind(address).await?;
    let actual_address = listener.local_addr()?;
    info!(%actual_address, "internal authorization listener started");
    Ok(Some(tokio::spawn(async move {
        axum::serve(listener, internal_router(Arc::new(state)))
            .with_graceful_shutdown(wait_for_shutdown(shutdown))
            .await
    })))
}

async fn await_background_tasks(
    worker: Option<WorkerHandle>,
    internal: Option<InternalServerHandle>,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(handle) = worker {
        handle.await??;
    }
    if let Some(handle) = internal {
        handle.await??;
    }
    Ok(())
}

fn load_cipher() -> Result<Option<EnvelopeCipher>, Box<dyn std::error::Error>> {
    match std::env::var("MWC_ENCRYPTION_KEY") {
        Ok(encoded_key) => Ok(Some(EnvelopeCipher::from_base64(&encoded_key)?)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

async fn kubernetes_runtime(
    config: &AppConfig,
    database: &Database,
    cipher: &Option<EnvelopeCipher>,
    enabled: bool,
) -> Result<(Option<kube::Client>, Option<WorkspaceReconcileHandler>), Box<dyn std::error::Error>> {
    if !enabled {
        return Ok((None, None));
    }
    let workspace_cipher = cipher.clone().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "MWC_ENCRYPTION_KEY is required when Kubernetes coordination is enabled",
        )
    })?;
    let client = kube::Client::try_default().await?;
    let builder = resource_builder(config)?;
    let coordinator = KubernetesCoordinator::new(client.clone(), builder.clone());
    let handler =
        WorkspaceReconcileHandler::new(database.clone(), workspace_cipher, builder, coordinator);
    Ok((Some(client), Some(handler)))
}

fn job_worker(
    config: &AppConfig,
    database: &Database,
    cipher: &Option<EnvelopeCipher>,
    workspace_handler: Option<WorkspaceReconcileHandler>,
    observability: memeloop_workspace_control::observability::Observability,
) -> Result<Option<JobWorker<ControlPlaneJobHandler>>, Box<dyn std::error::Error>> {
    let Some(worker_cipher) = cipher.clone() else {
        return Ok(None);
    };
    let webhook_handler =
        WebhookDeliveryHandler::new(database.clone(), worker_cipher, observability)?;
    let handler = Arc::new(ControlPlaneJobHandler::new(
        workspace_handler,
        webhook_handler,
    ));
    Ok(Some(JobWorker::new(
        database.clone(),
        handler,
        config.instance_id.clone(),
    )))
}

fn app_state(
    config: &AppConfig,
    database: Database,
    cipher: Option<EnvelopeCipher>,
    kubernetes_client: Option<kube::Client>,
    kubernetes_enabled: bool,
    diagnostics_enabled: bool,
    plugins: PluginRuntime,
) -> Result<AppState, Box<dyn std::error::Error>> {
    let mut state = match cipher {
        Some(cipher) => AppState::with_cipher(config.clone(), database, cipher),
        None => AppState::new(config.clone(), database),
    };
    if let Some(client) = kubernetes_client {
        state.set_kubernetes_client(client);
    }
    state.set_plugin_runtime(plugins);
    if diagnostics_enabled {
        state.enable_diagnostics();
    }
    if let Ok(public_key) = std::env::var("MWC_SSH_JUMP_HOST_PUBLIC_KEY") {
        state.set_jump_host_public_key(&public_key)?;
    }
    configure_internal_auth(&mut state, kubernetes_enabled || diagnostics_enabled)?;
    Ok(state)
}

fn configure_internal_auth(
    state: &mut AppState,
    kubernetes_enabled: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    match std::env::var("MWC_INTERNAL_AUTH_TOKEN") {
        Ok(token) if token.len() >= 32 => state.set_internal_auth_token(&token),
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "MWC_INTERNAL_AUTH_TOKEN must contain at least 32 bytes",
            )
            .into());
        }
        Err(std::env::VarError::NotPresent) if kubernetes_enabled => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "MWC_INTERNAL_AUTH_TOKEN is required when Kubernetes coordination is enabled",
            )
            .into());
        }
        Err(std::env::VarError::NotPresent) => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn internal_listener_address(
    kubernetes_enabled: bool,
    diagnostics_enabled: bool,
) -> Result<Option<std::net::SocketAddr>, Box<dyn std::error::Error>> {
    match std::env::var("MWC_INTERNAL_LISTEN_ADDRESS") {
        Ok(value) => Ok(Some(value.parse()?)),
        Err(std::env::VarError::NotPresent) if kubernetes_enabled => {
            Ok(Some("0.0.0.0:8081".parse()?))
        }
        Err(std::env::VarError::NotPresent) if diagnostics_enabled => {
            Ok(Some("127.0.0.1:8081".parse()?))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

async fn wait_for_shutdown(mut shutdown: tokio::sync::watch::Receiver<bool>) {
    while !*shutdown.borrow() {
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

fn resource_builder(
    config: &memeloop_workspace_control::config::AppConfig,
) -> Result<ResourceBuilder, io::Error> {
    let ttyd_image = std::env::var("MWC_TTYD_IMAGE").map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "MWC_TTYD_IMAGE is required when Kubernetes coordination is enabled",
        )
    })?;
    let higress_namespace =
        std::env::var("MWC_HIGRESS_NAMESPACE").unwrap_or_else(|_| "higress-system".to_owned());
    let jump_host_namespace = std::env::var("MWC_JUMP_HOST_NAMESPACE")
        .unwrap_or_else(|_| format!("mwc-{}", config.installation_id));
    let storage_class_name = std::env::var("MWC_STORAGE_CLASS_NAME")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let web_shell_domain = std::env::var("MWC_WEB_SHELL_DOMAIN")
        .ok()
        .filter(|value| !value.trim().is_empty());
    if web_shell_domain.is_some() && !env_bool("MWC_WEB_SHELL_AUTH_CONFIGURED", false)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "MWC_WEB_SHELL_AUTH_CONFIGURED=true is required before exposing ttyd routes",
        ));
    }
    if let Some(domain) = web_shell_domain.as_deref() {
        let expected_origin = format!("https://{domain}");
        if config.web_shell_public_origin.as_deref() != Some(expected_origin.as_str()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "MWC_WEB_SHELL_PUBLIC_ORIGIN must exactly match the configured Web Shell domain",
            ));
        }
    }
    let higress_pod_labels = match std::env::var("MWC_HIGRESS_POD_LABELS_JSON") {
        Ok(value) => {
            let labels: BTreeMap<String, String> =
                serde_json::from_str(&value).map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("MWC_HIGRESS_POD_LABELS_JSON must be a JSON string map: {error}"),
                    )
                })?;
            if labels.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "MWC_HIGRESS_POD_LABELS_JSON must not be empty",
                ));
            }
            labels
        }
        Err(std::env::VarError::NotPresent) => BTreeMap::from([(
            "app.kubernetes.io/name".to_owned(),
            "higress-gateway".to_owned(),
        )]),
        Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidInput, error)),
    };
    let higress_source_cidrs = match std::env::var("MWC_HIGRESS_SOURCE_CIDRS_JSON") {
        Ok(value) => {
            let cidrs: Vec<String> = serde_json::from_str(&value).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("MWC_HIGRESS_SOURCE_CIDRS_JSON must be a JSON string array: {error}"),
                )
            })?;
            if cidrs.iter().any(|cidr| cidr.trim().is_empty()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "MWC_HIGRESS_SOURCE_CIDRS_JSON must not contain empty CIDRs",
                ));
            }
            cidrs
        }
        Err(std::env::VarError::NotPresent) => Vec::new(),
        Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidInput, error)),
    };
    Ok(ResourceBuilder {
        installation_id: config.installation_id.clone(),
        ttyd_image,
        higress_namespace,
        higress_pod_labels,
        higress_source_cidrs,
        jump_host_namespace: jump_host_namespace.clone(),
        jump_host_pod_labels: BTreeMap::from([(
            "app.kubernetes.io/name".to_owned(),
            "mwc-ssh-jump".to_owned(),
        )]),
        storage_class_name,
        web_shell_domain,
        port_mapping_domain: config.port_mapping_public_domain.clone(),
        higress_gateway_name: std::env::var("MWC_HIGRESS_GATEWAY_NAME")
            .unwrap_or_else(|_| "higress-gateway".to_owned()),
        higress_https_section_name: std::env::var("MWC_HIGRESS_HTTPS_SECTION_NAME")
            .unwrap_or_else(|_| "https".to_owned()),
        internal_ssh_node_port_enabled: config.internal_ssh_host.is_some(),
    })
}

fn env_bool(name: &'static str, default: bool) -> Result<bool, io::Error> {
    match std::env::var(name) {
        Ok(value) if matches!(value.as_str(), "1" | "true") => Ok(true),
        Ok(value) if matches!(value.as_str(), "0" | "false") => Ok(false),
        Ok(value) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} must be true, false, 1, or 0; got {value}"),
        )),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(io::Error::new(io::ErrorKind::InvalidInput, error)),
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
