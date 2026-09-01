use std::{fmt, net::SocketAddr, path::PathBuf, str::FromStr};

use clap::Args;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;
use utoipa::ToSchema;

const MAX_INSTALLATION_ID_LENGTH: usize = 20;

#[derive(Debug, Clone, Args)]
pub struct AppConfig {
    /// Immutable identifier used to isolate every managed Kubernetes resource.
    #[arg(long, env = "MWC_INSTALLATION_ID")]
    pub installation_id: InstallationId,

    #[arg(long, env = "MWC_LISTEN_ADDRESS", default_value = "0.0.0.0:8080")]
    pub listen_address: SocketAddr,

    #[arg(
        long,
        env = "MWC_DATABASE_URL",
        default_value = "sqlite://data/control-plane.sqlite?mode=rwc"
    )]
    pub database_url: String,

    /// Expected application replica count. SQLite deliberately rejects values above one.
    #[arg(long, env = "MWC_REPLICA_COUNT", default_value_t = 1)]
    pub replica_count: u16,

    /// Stable identity used as a database task lease owner.
    #[arg(long, env = "MWC_INSTANCE_ID", default_value = "local")]
    pub instance_id: String,

    /// Public OpenSSH jump-host DNS name. Omit for an internal-only installation.
    #[arg(long, env = "MWC_SSH_PUBLIC_HOST")]
    pub ssh_public_host: Option<String>,

    /// Stable Tailnet node address used with Kubernetes-assigned workspace SSH NodePorts.
    /// When omitted, internal SSH remains available only through ClusterIP DNS.
    #[arg(long, env = "MWC_INTERNAL_SSH_HOST")]
    pub internal_ssh_host: Option<String>,

    /// Public origin used for ttyd links, for example https://shell.example.com.
    #[arg(long, env = "MWC_WEB_SHELL_PUBLIC_ORIGIN")]
    pub web_shell_public_origin: Option<String>,

    /// DNS suffix used for authenticated workspace HTTP mappings, for example
    /// `k3s.example.com` for `p-<mapping-id>.k3s.example.com`.
    #[arg(long, env = "MWC_PORT_MAPPING_PUBLIC_DOMAIN")]
    pub port_mapping_public_domain: Option<String>,

    /// Prometheus base URL used for PVC usage telemetry. Omit to disable storage telemetry.
    #[arg(long, env = "MWC_PROMETHEUS_URL")]
    pub prometheus_url: Option<Url>,

    /// Read-only root whose immediate child directories are plugin packages.
    /// Package changes take effect only after a process restart.
    #[arg(long, env = "MWC_PLUGIN_DIR")]
    pub plugin_dir: Option<PathBuf>,
}

impl AppConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        let database_mode = DatabaseMode::from_url(&self.database_url)?;
        if self.replica_count == 0 {
            return Err(ConfigError::ZeroReplicas);
        }
        if database_mode == DatabaseMode::Sqlite && self.replica_count != 1 {
            return Err(ConfigError::SqliteMultipleReplicas(self.replica_count));
        }
        if self.instance_id.trim().is_empty() {
            return Err(ConfigError::EmptyInstanceId);
        }
        if self.ssh_public_host.as_deref().is_some_and(|host| {
            host.is_empty()
                || host.len() > 253
                || !host.chars().all(|character| {
                    character.is_ascii_lowercase()
                        || character.is_ascii_digit()
                        || ".-".contains(character)
                })
        }) {
            return Err(ConfigError::InvalidSshPublicHost);
        }
        if self
            .internal_ssh_host
            .as_deref()
            .is_some_and(|host| !valid_ssh_host(host))
        {
            return Err(ConfigError::InvalidInternalSshHost);
        }
        if self
            .web_shell_public_origin
            .as_deref()
            .is_some_and(|origin| {
                !origin.starts_with("https://")
                    || origin.contains(['\r', '\n', '?', '#'])
                    || origin.ends_with('/')
            })
        {
            return Err(ConfigError::InvalidWebShellOrigin);
        }
        if self
            .port_mapping_public_domain
            .as_deref()
            .is_some_and(|domain| {
                domain.parse::<std::net::IpAddr>().is_ok()
                    || !domain.contains('.')
                    || !valid_ssh_host(domain)
            })
        {
            return Err(ConfigError::InvalidPortMappingDomain);
        }
        if self.prometheus_url.as_ref().is_some_and(|url| {
            !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
        }) {
            return Err(ConfigError::InvalidPrometheusUrl);
        }
        Ok(())
    }

    pub fn database_mode(&self) -> Result<DatabaseMode, ConfigError> {
        DatabaseMode::from_url(&self.database_url)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseMode {
    Sqlite,
    Postgres,
}

impl DatabaseMode {
    pub fn from_url(url: &str) -> Result<Self, ConfigError> {
        if url.starts_with("sqlite:") {
            Ok(Self::Sqlite)
        } else if url.starts_with("postgres:") || url.starts_with("postgresql:") {
            Ok(Self::Postgres)
        } else {
            Err(ConfigError::UnsupportedDatabaseUrl)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String, example = "internal-a")]
pub struct InstallationId(String);

impl InstallationId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn workspace_namespace(&self, workspace_short_id: &str) -> Result<String, ConfigError> {
        validate_dns_label(workspace_short_id, 30, "workspace short id")?;
        Ok(format!("ws-{}-{workspace_short_id}", self.0))
    }
}

impl fmt::Display for InstallationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for InstallationId {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        validate_dns_label(value, MAX_INSTALLATION_ID_LENGTH, "installation id")?;
        Ok(Self(value.to_owned()))
    }
}

fn validate_dns_label(
    value: &str,
    max_length: usize,
    field: &'static str,
) -> Result<(), ConfigError> {
    let valid_edge = |character: char| character.is_ascii_lowercase() || character.is_ascii_digit();
    let valid_body = |character: char| valid_edge(character) || character == '-';

    if value.is_empty()
        || value.len() > max_length
        || !value.chars().all(valid_body)
        || !value.chars().next().is_some_and(valid_edge)
        || !value.chars().last().is_some_and(valid_edge)
    {
        return Err(ConfigError::InvalidDnsLabel {
            field,
            value: value.to_owned(),
            max_length,
        });
    }
    Ok(())
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("{field} must be a lower-case DNS label of at most {max_length} characters: {value}")]
    InvalidDnsLabel {
        field: &'static str,
        value: String,
        max_length: usize,
    },
    #[error("database URL must use sqlite:, postgres:, or postgresql:")]
    UnsupportedDatabaseUrl,
    #[error("replica count must be at least one")]
    ZeroReplicas,
    #[error("SQLite mode requires exactly one application replica, got {0}")]
    SqliteMultipleReplicas(u16),
    #[error("instance id must not be empty")]
    EmptyInstanceId,
    #[error("SSH public host must be a lower-case DNS hostname")]
    InvalidSshPublicHost,
    #[error("internal SSH host must be an IP address or lower-case DNS hostname")]
    InvalidInternalSshHost,
    #[error("Web Shell public origin must be an https origin without a trailing slash")]
    InvalidWebShellOrigin,
    #[error("port mapping public domain must be a lower-case DNS suffix")]
    InvalidPortMappingDomain,
    #[error("Prometheus URL must be an http(s) base URL without credentials, query, or fragment")]
    InvalidPrometheusUrl,
}

fn valid_ssh_host(host: &str) -> bool {
    if host.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    !host.is_empty()
        && host.len() <= 253
        && !host.starts_with(['.', '-'])
        && !host.ends_with(['.', '-'])
        && host.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installation_id_builds_isolated_namespace() {
        let installation = "public-a".parse::<InstallationId>().unwrap();
        assert_eq!(
            installation.workspace_namespace("01jabc").unwrap(),
            "ws-public-a-01jabc"
        );
    }

    #[test]
    fn rejects_values_that_are_not_dns_labels() {
        for invalid in ["", "Public", "-public", "public-", "public_a"] {
            assert!(invalid.parse::<InstallationId>().is_err(), "{invalid}");
        }
    }

    #[test]
    fn sqlite_rejects_horizontal_scaling() {
        let config = AppConfig {
            installation_id: "test".parse().unwrap(),
            listen_address: "127.0.0.1:8080".parse().unwrap(),
            database_url: "sqlite::memory:".to_owned(),
            replica_count: 2,
            instance_id: "one".to_owned(),
            ssh_public_host: None,
            internal_ssh_host: None,
            web_shell_public_origin: None,
            port_mapping_public_domain: None,
            prometheus_url: None,
            plugin_dir: None,
        };
        assert_eq!(
            config.validate(),
            Err(ConfigError::SqliteMultipleReplicas(2))
        );
    }

    #[test]
    fn internal_ssh_host_accepts_tailnet_addresses_and_rejects_shell_text() {
        let base = AppConfig {
            installation_id: "test".parse().unwrap(),
            listen_address: "127.0.0.1:8080".parse().unwrap(),
            database_url: "sqlite::memory:".to_owned(),
            replica_count: 1,
            instance_id: "one".to_owned(),
            ssh_public_host: None,
            internal_ssh_host: Some("100.64.12.34".to_owned()),
            web_shell_public_origin: None,
            port_mapping_public_domain: None,
            prometheus_url: None,
            plugin_dir: None,
        };
        assert!(base.validate().is_ok());
        assert!(
            AppConfig {
                internal_ssh_host: Some("workspace-node.tailnet.example".to_owned()),
                ..base.clone()
            }
            .validate()
            .is_ok()
        );
        assert_eq!(
            AppConfig {
                internal_ssh_host: Some("node;touch /tmp/no".to_owned()),
                ..base
            }
            .validate(),
            Err(ConfigError::InvalidInternalSshHost)
        );
    }

    #[test]
    fn prometheus_url_accepts_safe_base_urls_only() {
        let base = AppConfig {
            installation_id: "test".parse().unwrap(),
            listen_address: "127.0.0.1:8080".parse().unwrap(),
            database_url: "sqlite::memory:".to_owned(),
            replica_count: 1,
            instance_id: "one".to_owned(),
            ssh_public_host: None,
            internal_ssh_host: None,
            web_shell_public_origin: None,
            port_mapping_public_domain: None,
            prometheus_url: Some(
                "http://prometheus.monitoring.svc:9090/prometheus"
                    .parse()
                    .unwrap(),
            ),
            plugin_dir: None,
        };
        assert!(base.validate().is_ok());
        for invalid in [
            "ftp://prometheus.example",
            "http://user:password@prometheus.example",
            "http://prometheus.example?query=up",
            "http://prometheus.example/#fragment",
        ] {
            assert_eq!(
                AppConfig {
                    prometheus_url: Some(invalid.parse().unwrap()),
                    ..base.clone()
                }
                .validate(),
                Err(ConfigError::InvalidPrometheusUrl),
                "{invalid}"
            );
        }
    }
}
