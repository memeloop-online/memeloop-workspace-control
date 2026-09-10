use std::{collections::BTreeSet, net::IpAddr, time::Duration};

use reqwest::{Client, redirect::Policy};
use serde::Deserialize;
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
    client: Client,
    config: DynamicEgressRefreshConfig,
    egress: InternetEgressConfig,
}

impl DynamicEgressRefresh {
    pub fn new(
        config: DynamicEgressRefreshConfig,
        egress: InternetEgressConfig,
    ) -> Result<Self, DynamicEgressRefreshError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(Policy::none())
            .build()
            .map_err(|error| DynamicEgressRefreshError::Dns(error.to_string()))?;
        Ok(Self {
            client,
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
        let (ipv4, ipv6) = tokio::join!(
            self.resolve_record_type(hostname, 1),
            self.resolve_record_type(hostname, 28)
        );
        let addresses = [ipv4, ipv6]
            .into_iter()
            .filter_map(Result::ok)
            .flatten()
            .collect::<Vec<_>>();
        if addresses.is_empty() {
            return Err(DynamicEgressRefreshError::Dns(
                "hostname has no A or AAAA records".to_owned(),
            ));
        }
        Ok(addresses)
    }

    async fn resolve_record_type(
        &self,
        hostname: &str,
        record_type: u16,
    ) -> Result<Vec<IpAddr>, DynamicEgressRefreshError> {
        let mut last_error = None;
        for server in &self.config.public_dns_servers {
            match self.query_server(*server, hostname, record_type).await {
                Ok(addresses) => return Ok(addresses),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| {
            DynamicEgressRefreshError::Dns("no public DNS server is configured".to_owned())
        }))
    }

    async fn query_server(
        &self,
        server: IpAddr,
        hostname: &str,
        record_type: u16,
    ) -> Result<Vec<IpAddr>, DynamicEgressRefreshError> {
        let endpoint = match server {
            IpAddr::V4(address) => format!("https://{address}/dns-query"),
            IpAddr::V6(address) => format!("https://[{address}]/dns-query"),
        };
        let record_type_query = record_type.to_string();
        let response = self
            .client
            .get(endpoint)
            .header("accept", "application/dns-json")
            .query(&[("name", hostname), ("type", record_type_query.as_str())])
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| DynamicEgressRefreshError::Dns(error.to_string()))?
            .json::<DnsJsonResponse>()
            .await
            .map_err(|error| DynamicEgressRefreshError::Dns(error.to_string()))?;
        if response.status != 0 {
            return Err(DynamicEgressRefreshError::Dns(format!(
                "resolver returned DNS status {}",
                response.status
            )));
        }
        Ok(dns_json_addresses(response.answers, record_type))
    }
}

#[derive(Debug, Deserialize)]
struct DnsJsonResponse {
    #[serde(rename = "Status")]
    status: u16,
    #[serde(rename = "Answer", default)]
    answers: Vec<DnsJsonAnswer>,
}

#[derive(Debug, Deserialize)]
struct DnsJsonAnswer {
    #[serde(rename = "type")]
    record_type: u16,
    data: String,
}

fn dns_json_addresses(answers: Vec<DnsJsonAnswer>, record_type: u16) -> Vec<IpAddr> {
    answers
        .into_iter()
        .filter(|answer| answer.record_type == record_type)
        .filter_map(|answer| answer.data.parse().ok())
        .collect()
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

    #[test]
    fn dns_json_keeps_ipv4_and_ipv6_answers() {
        let ipv4: DnsJsonResponse =
            serde_json::from_str(r#"{"Status":0,"Answer":[{"type":1,"data":"192.0.2.9"}]}"#)
                .unwrap();
        let ipv6: DnsJsonResponse =
            serde_json::from_str(r#"{"Status":0,"Answer":[{"type":28,"data":"2001:db8::9"}]}"#)
                .unwrap();
        assert_eq!(
            dns_json_addresses(ipv4.answers, 1),
            vec!["192.0.2.9".parse::<IpAddr>().unwrap()]
        );
        assert_eq!(
            dns_json_addresses(ipv6.answers, 28),
            vec!["2001:db8::9".parse::<IpAddr>().unwrap()]
        );
    }
}
