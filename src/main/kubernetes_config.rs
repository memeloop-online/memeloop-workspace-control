use std::{collections::BTreeMap, io};

use ipnet::IpNet;
use memeloop_workspace_control::{
    config::AppConfig,
    kubernetes::{InternetEgressConfig, ResourceBuilder},
};

pub(super) fn resource_builder(config: &AppConfig) -> Result<ResourceBuilder, io::Error> {
    let ttyd_image = std::env::var("MWC_TTYD_IMAGE").map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "MWC_TTYD_IMAGE is required when Kubernetes coordination is enabled",
        )
    })?;
    let higress_namespace =
        std::env::var("MWC_HIGRESS_NAMESPACE").unwrap_or_else(|_| "higress-system".to_owned());
    let jump_host_namespace = std::env::var("MWC_JUMP_HOST_NAMESPACE").unwrap_or_else(|_| {
        memeloop_workspace_control::workspace_runtime::WORKSPACE_NAMESPACE.to_owned()
    });
    let storage_class_name = std::env::var("MWC_STORAGE_CLASS_NAME")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let web_shell_domain = std::env::var("MWC_WEB_SHELL_DOMAIN")
        .ok()
        .filter(|value| !value.trim().is_empty());
    if web_shell_domain.is_some() && !super::env_bool("MWC_WEB_SHELL_AUTH_CONFIGURED", false)? {
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
    let internet_egress = internet_egress_config()?;
    Ok(ResourceBuilder {
        installation_id: config.installation_id.clone(),
        ttyd_image,
        higress_namespace,
        higress_pod_labels,
        higress_source_cidrs,
        internet_egress,
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

fn internet_egress_config() -> Result<Option<InternetEgressConfig>, io::Error> {
    let namespace = optional_env("MWC_EGRESS_DNS_NAMESPACE")?;
    let labels = optional_json_map("MWC_EGRESS_DNS_POD_LABELS_JSON")?;
    let blocked = optional_cidrs("MWC_EGRESS_ADDITIONAL_BLOCKED_CIDRS_JSON")?;
    match (namespace, labels) {
        (None, None) if blocked.is_none() => Ok(None),
        (Some(namespace), Some(labels)) => {
            InternetEgressConfig::new(namespace, labels, blocked.unwrap_or_default())
                .map(Some)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "MWC_EGRESS_DNS_NAMESPACE and MWC_EGRESS_DNS_POD_LABELS_JSON must be set together when configuring internet-only egress",
        )),
    }
}

fn optional_env(name: &'static str) -> Result<Option<String>, io::Error> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(io::Error::new(io::ErrorKind::InvalidInput, error)),
    }
}

fn optional_json_map(name: &'static str) -> Result<Option<BTreeMap<String, String>>, io::Error> {
    let Some(value) = optional_env(name)? else {
        return Ok(None);
    };
    serde_json::from_str(&value).map(Some).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} must be a JSON string map: {error}"),
        )
    })
}

fn optional_cidrs(name: &'static str) -> Result<Option<Vec<IpNet>>, io::Error> {
    let Some(value) = optional_env(name)? else {
        return Ok(None);
    };
    serde_json::from_str::<Vec<String>>(&value)
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{name} must be a JSON string array: {error}"),
            )
        })?
        .into_iter()
        .map(|cidr| {
            cidr.parse::<IpNet>().map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{name} has invalid CIDR {cidr:?}: {error}"),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}
