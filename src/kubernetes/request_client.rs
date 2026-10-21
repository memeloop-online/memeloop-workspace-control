use std::time::Duration;

use kube::{Client, Config};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const READ_TIMEOUT: Duration = Duration::from_secs(15);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// Build the client for finite control-plane requests. Long-lived Kubernetes
/// watch/exec/attach streams must use a separately configured client.
pub fn request_client(mut config: Config) -> Result<Client, kube::Error> {
    config.connect_timeout = Some(CONNECT_TIMEOUT);
    config.read_timeout = Some(READ_TIMEOUT);
    config.write_timeout = Some(WRITE_TIMEOUT);
    // Persisted jobs already schedule retries. Hidden SDK retries, especially
    // Retry-After responses, would keep an executor occupied and unobservable.
    config.default_retry = false;
    let client = Client::try_from(config)?;
    tracing::info!(
        connect_timeout_seconds = CONNECT_TIMEOUT.as_secs(),
        read_timeout_seconds = READ_TIMEOUT.as_secs(),
        write_timeout_seconds = WRITE_TIMEOUT.as_secs(),
        sdk_retries = false,
        "configured finite Kubernetes API requests"
    );
    Ok(client)
}

/// Watches have a server-side 60-second lifetime. Keep their transport separate
/// so finite reconciliation calls still release their executor after 15 seconds.
pub fn watch_client(mut config: Config) -> Result<Client, kube::Error> {
    config.connect_timeout = Some(CONNECT_TIMEOUT);
    config.read_timeout = Some(Duration::from_secs(75));
    config.write_timeout = Some(WRITE_TIMEOUT);
    config.default_retry = false;
    Client::try_from(config)
}

#[cfg(test)]
#[path = "request_client_tests.rs"]
mod tests;
