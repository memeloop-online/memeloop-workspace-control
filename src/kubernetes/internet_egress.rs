use std::{
    collections::BTreeMap,
    net::IpAddr,
    sync::{Arc, RwLock},
};

use ipnet::IpNet;
use thiserror::Error;

use super::ttyd_mtls::valid_dns_label;

/// Operator-provided DNS identity and exceptional blocks for `internet_only` templates.
///
/// DNS is selected by its configured namespace and Pod labels, rather than a presumed service
/// IP. Operators must add public node addresses and nonstandard Pod/Service CIDRs to
/// `additional_blocked_cidrs`.
#[derive(Debug, Clone)]
pub struct InternetEgressConfig {
    pub dns_namespace: String,
    pub dns_pod_labels: BTreeMap<String, String>,
    pub additional_blocked_cidrs: Vec<IpNet>,
    observed_blocked_cidrs: Arc<RwLock<Vec<IpNet>>>,
}

impl InternetEgressConfig {
    pub fn new(
        dns_namespace: String,
        dns_pod_labels: BTreeMap<String, String>,
        additional_blocked_cidrs: Vec<IpNet>,
    ) -> Result<Self, InternetEgressConfigError> {
        let config = Self {
            dns_namespace,
            dns_pod_labels,
            additional_blocked_cidrs,
            observed_blocked_cidrs: Arc::new(RwLock::new(Vec::new())),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn remember_blocked_addresses(&self, addresses: impl IntoIterator<Item = IpAddr>) -> usize {
        let mut observed = self
            .observed_blocked_cidrs
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let before = observed.len();
        for address in addresses {
            let cidr = IpNet::from(address);
            if !self.additional_blocked_cidrs.contains(&cidr) && !observed.contains(&cidr) {
                observed.push(cidr);
            }
        }
        observed.sort_unstable_by_key(ToString::to_string);
        observed.len() - before
    }

    pub fn blocked_cidrs(&self) -> Vec<IpNet> {
        let observed = self
            .observed_blocked_cidrs
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut blocked = self.additional_blocked_cidrs.clone();
        blocked.extend(observed.iter().copied());
        blocked
    }

    pub fn validate(&self) -> Result<(), InternetEgressConfigError> {
        if !valid_dns_label(&self.dns_namespace) {
            return Err(InternetEgressConfigError::DnsNamespace);
        }
        if self.dns_pod_labels.is_empty()
            || self.dns_pod_labels.iter().any(|(key, value)| {
                key.is_empty()
                    || key.len() > 253
                    || value.len() > 63
                    || key.chars().any(char::is_whitespace)
                    || value.chars().any(char::is_whitespace)
            })
        {
            return Err(InternetEgressConfigError::DnsPodLabels);
        }
        Ok(())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InternetEgressConfigError {
    #[error("egress DNS namespace must be a lower-case DNS label")]
    DnsNamespace,
    #[error("egress DNS Pod labels must be non-empty and contain no whitespace")]
    DnsPodLabels,
}
