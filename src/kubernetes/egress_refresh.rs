use std::{collections::BTreeSet, net::IpAddr, time::Duration};

use hickory_resolver::{
    TokioResolver,
    config::{LookupIpStrategy, NameServerConfig, ResolverConfig, ResolverOpts},
    net::runtime::TokioRuntimeProvider,
    proto::rr::{RData, RecordType},
};
use thiserror::Error;
use tokio::sync::watch;
use tracing::{info, warn};

use crate::storage::Database;

use super::{InternetEgressConfig, KubernetesCoordinator};

#[derive(Debug, Clone)]
pub struct DynamicEgressRefreshConfig {
    pub hostnames: Vec<String>,
    pub public_dns_servers: Vec<IpAddr>,
    pub interval: Duration,
}

impl DynamicEgressRefreshConfig {
    pub fn new(
        hostnames: Vec<String>,
        public_dns_servers: Vec<IpAddr>,
        interval: Duration,
    ) -> Result<Self, DynamicEgressRefreshError> {
        if hostnames.is_empty() || hostnames.iter().any(|hostname| !valid_hostname(hostname)) {
            return Err(DynamicEgressRefreshError::InvalidHostnames);
        }
        if public_dns_servers.is_empty()
            || public_dns_servers
                .iter()
                .any(|address| address.is_unspecified() || address.is_multicast())
        {
            return Err(DynamicEgressRefreshError::InvalidResolverAddresses);
        }
        if interval < Duration::from_secs(30) {
            return Err(DynamicEgressRefreshError::RefreshInterval);
        }
        Ok(Self {
            hostnames,
            public_dns_servers,
            interval,
        })
    }
}

#[derive(Clone)]
pub struct DynamicEgressRefresh {
    resolver: TokioResolver,
    config: DynamicEgressRefreshConfig,
    egress: InternetEgressConfig,
}

impl DynamicEgressRefresh {
    pub fn new(
        config: DynamicEgressRefreshConfig,
        egress: InternetEgressConfig,
    ) -> Result<Self, DynamicEgressRefreshError> {
        let name_servers = config
            .public_dns_servers
            .iter()
            .copied()
            .map(NameServerConfig::tcp)
            .collect();
        let mut options = ResolverOpts::default();
        options.timeout = Duration::from_secs(5);
        options.attempts = 2;
        options.ip_strategy = LookupIpStrategy::Ipv4AndIpv6;
        let resolver = TokioResolver::builder_with_config(
            ResolverConfig::from_name_servers(name_servers),
            TokioRuntimeProvider::default(),
        )
        .with_options(options)
        .build()
        .map_err(|error| DynamicEgressRefreshError::Dns(error.to_string()))?;
        Ok(Self {
            resolver,
            config,
            egress,
        })
    }

    pub async fn refresh_addresses(&self) -> Result<RefreshResult, DynamicEgressRefreshError> {
        let mut addresses = BTreeSet::new();
        let mut successful_queries = 0_u64;
        let mut failed_queries = 0_u64;
        for hostname in &self.config.hostnames {
            match self.resolve(hostname).await {
                Ok(resolved) => {
                    successful_queries += 1;
                    addresses.extend(resolved);
                }
                Err(error) => {
                    failed_queries += 1;
                    warn!(%hostname, error = %error, "public DNS query failed");
                }
            }
        }
        if successful_queries == 0 {
            return Err(DynamicEgressRefreshError::NoSuccessfulQueries);
        }
        let resolved_addresses = addresses.len();
        let added_addresses = self.egress.remember_blocked_addresses(addresses);
        Ok(RefreshResult {
            resolved_addresses,
            added_addresses,
            successful_queries,
            failed_queries,
        })
    }

    pub async fn run(
        self,
        database: Database,
        coordinator: KubernetesCoordinator,
        mut shutdown: watch::Receiver<bool>,
    ) {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(self.config.interval) => {
                    self.refresh_policies_once(&database, &coordinator).await;
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return;
                    }
                }
            }
        }
    }

    async fn refresh_policies_once(
        &self,
        database: &Database,
        coordinator: &KubernetesCoordinator,
    ) {
        match self.refresh_addresses().await {
            Ok(result) => Self::apply_refresh_result(result, database, coordinator).await,
            Err(error) => {
                warn!(error = %error, "dynamic workspace egress resolution failed");
            }
        }
    }

    async fn apply_refresh_result(
        result: RefreshResult,
        database: &Database,
        coordinator: &KubernetesCoordinator,
    ) {
        if result.added_addresses == 0 {
            info!(
                resolved_addresses = result.resolved_addresses,
                failed_queries = result.failed_queries,
                "dynamic workspace egress blocks unchanged"
            );
            return;
        }
        let refreshed = coordinator.refresh_network_policies(database).await;
        match refreshed {
            Ok(updated) => info!(
                added_addresses = result.added_addresses,
                resolved_addresses = result.resolved_addresses,
                updated,
                "dynamic workspace egress blocks refreshed"
            ),
            Err(error) => warn!(error = %error, "dynamic workspace policy refresh failed"),
        }
    }

    async fn resolve(&self, hostname: &str) -> Result<Vec<IpAddr>, DynamicEgressRefreshError> {
        let hostname = format!("{hostname}.");
        let (ipv4, ipv6) = tokio::join!(
            self.resolver.lookup(hostname.clone(), RecordType::A),
            self.resolver.lookup(hostname, RecordType::AAAA)
        );
        let addresses = [ipv4.ok(), ipv6.ok()]
            .into_iter()
            .flatten()
            .flat_map(|lookup| {
                lookup
                    .answers()
                    .iter()
                    .filter_map(|record| match &record.data {
                        RData::A(address) => Some(IpAddr::V4(address.0)),
                        RData::AAAA(address) => Some(IpAddr::V6(address.0)),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        if addresses.is_empty() {
            return Err(DynamicEgressRefreshError::Dns(
                "hostname has no A or AAAA records".to_owned(),
            ));
        }
        Ok(addresses)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefreshResult {
    pub resolved_addresses: usize,
    pub added_addresses: usize,
    pub successful_queries: u64,
    pub failed_queries: u64,
}

#[derive(Debug, Error)]
pub enum DynamicEgressRefreshError {
    #[error("dynamic egress hostnames must be non-empty lower-case DNS names")]
    InvalidHostnames,
    #[error("public DNS resolver addresses must contain at least one usable IP address")]
    InvalidResolverAddresses,
    #[error("dynamic egress refresh interval must be at least 30 seconds")]
    RefreshInterval,
    #[error("every public DNS query failed")]
    NoSuccessfulQueries,
    #[error("public DNS query failed: {0}")]
    Dns(String),
}

fn valid_hostname(hostname: &str) -> bool {
    hostname.len() <= 253
        && hostname.contains('.')
        && hostname.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                && label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
        })
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, time::Duration};

    use super::*;

    #[test]
    fn dynamic_configuration_rejects_unsafe_inputs() {
        let resolver = vec!["1.1.1.1".parse().unwrap()];
        assert!(
            DynamicEgressRefreshConfig::new(
                vec!["edge.example.com".to_owned()],
                resolver,
                Duration::from_secs(30),
            )
            .is_ok()
        );
        for hostname in ["UPPER.example", "localhost", "-bad.example", "bad..example"] {
            assert!(
                DynamicEgressRefreshConfig::new(
                    vec![hostname.to_owned()],
                    vec!["1.1.1.1".parse().unwrap()],
                    Duration::from_secs(30),
                )
                .is_err()
            );
        }
        assert!(
            DynamicEgressRefreshConfig::new(
                vec!["edge.example.com".to_owned()],
                vec!["0.0.0.0".parse().unwrap()],
                Duration::from_secs(30),
            )
            .is_err()
        );
    }

    #[test]
    fn observed_addresses_are_monotonic_and_shared() {
        let egress = InternetEgressConfig::new(
            "kube-system".to_owned(),
            BTreeMap::from([("k8s-app".to_owned(), "kube-dns".to_owned())]),
            vec!["203.0.113.4/32".parse().unwrap()],
        )
        .unwrap();
        let clone = egress.clone();
        assert_eq!(
            egress.remember_blocked_addresses([
                "203.0.113.4".parse().unwrap(),
                "198.51.100.9".parse().unwrap(),
            ]),
            1
        );
        assert_eq!(
            clone.remember_blocked_addresses(["198.51.100.9".parse().unwrap()]),
            0
        );
        assert_eq!(clone.blocked_cidrs().len(), 2);
    }
}
