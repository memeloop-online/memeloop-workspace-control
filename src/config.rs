use std::{fmt, net::SocketAddr, str::FromStr};

use clap::Args;
use serde::{Deserialize, Serialize};
use thiserror::Error;
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

    /// Public origin used for ttyd links, for example https://shell.example.com.
    #[arg(long, env = "MWC_WEB_SHELL_PUBLIC_ORIGIN")]
    pub web_shell_public_origin: Option<String>,
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
    #[error("Web Shell public origin must be an https origin without a trailing slash")]
    InvalidWebShellOrigin,
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
            web_shell_public_origin: None,
        };
        assert_eq!(
            config.validate(),
            Err(ConfigError::SqliteMultipleReplicas(2))
        );
    }
}
