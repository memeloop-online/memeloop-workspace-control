use std::{collections::BTreeMap, io, time::Duration};

use ipnet::IpNet;
use memeloop_workspace_control::{
    config::AppConfig,
    kubernetes::{
        DynamicEgressRefresh, DynamicEgressRefreshConfig, InternetEgressConfig, ResourceBuilder,
        TtydMtlsConfig,
    },
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
        ttyd_mtls: ttyd_mtls_config(&higress_namespace)?,
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

pub(super) fn dynamic_egress_refresh(
    builder: &ResourceBuilder,
) -> Result<Option<DynamicEgressRefresh>, io::Error> {
    let hostnames = optional_json_strings("MWC_EGRESS_DYNAMIC_BLOCKED_HOSTS_JSON")?;
    let Some(hostnames) = hostnames.filter(|hostnames| !hostnames.is_empty()) else {
        return Ok(None);
    };
    let egress = builder.internet_egress.clone().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "dynamic blocked hosts require internet-only egress configuration",
        )
    })?;
    let public_dns_servers = optional_json_strings("MWC_EGRESS_PUBLIC_DNS_SERVERS_JSON")?
        .unwrap_or_else(|| vec!["1.1.1.1".to_owned(), "1.0.0.1".to_owned()]);
    let public_dns_servers = public_dns_servers
        .into_iter()
        .map(|address| {
            address
                .parse()
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let interval = std::env::var("MWC_EGRESS_DYNAMIC_REFRESH_SECONDS")
        .unwrap_or_else(|_| "300".to_owned())
        .parse::<u64>()
        .map(Duration::from_secs)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let config = DynamicEgressRefreshConfig::new(hostnames, public_dns_servers, interval)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    DynamicEgressRefresh::new(config, egress)
        .map(Some)
        .map_err(|error| io::Error::other(error))
}

fn ttyd_mtls_config(gateway_namespace: &str) -> Result<Option<TtydMtlsConfig>, io::Error> {
    let server = optional_env("MWC_TTYD_MTLS_SERVER_SECRET")?;
    let client_ca = optional_env("MWC_TTYD_MTLS_CLIENT_CA_SECRET")?;
    let namespace = optional_env("MWC_HIGRESS_MTLS_CLIENT_SECRET_NAMESPACE")?;
    let client = optional_env("MWC_HIGRESS_MTLS_CLIENT_SECRET_NAME")?;
    parse_ttyd_mtls(server, client_ca, namespace, client, gateway_namespace)
}

fn parse_ttyd_mtls(
    server: Option<String>,
    client_ca: Option<String>,
    namespace: Option<String>,
    client: Option<String>,
    gateway_namespace: &str,
) -> Result<Option<TtydMtlsConfig>, io::Error> {
    match (server, client_ca, namespace, client) {
        (None, None, None, None) => Ok(None),
        (Some(server), Some(client_ca), Some(namespace), Some(client)) => {
            if namespace != gateway_namespace {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "MWC_HIGRESS_MTLS_CLIENT_SECRET_NAMESPACE must equal MWC_HIGRESS_NAMESPACE",
                ));
            }
            TtydMtlsConfig::new(server, client_ca, namespace, client)
                .map(Some)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "MWC_TTYD_MTLS_SERVER_SECRET, MWC_TTYD_MTLS_CLIENT_CA_SECRET, MWC_HIGRESS_MTLS_CLIENT_SECRET_NAMESPACE and MWC_HIGRESS_MTLS_CLIENT_SECRET_NAME must be configured together",
        )),
    }
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

fn optional_json_strings(name: &'static str) -> Result<Option<Vec<String>>, io::Error> {
    let Some(value) = optional_env(name)? else {
        return Ok(None);
    };
    serde_json::from_str::<Vec<String>>(&value)
        .map(Some)
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{name} must be a JSON string array: {error}"),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::parse_ttyd_mtls;

    #[test]
    fn mtls_configuration_requires_all_four_references() {
        for mask in 0..16 {
            let result = parse_ttyd_mtls(
                (mask & 1 != 0).then(|| "ttyd-server".to_owned()),
                (mask & 2 != 0).then(|| "ttyd-client-ca".to_owned()),
                (mask & 4 != 0).then(|| "higress-system".to_owned()),
                (mask & 8 != 0).then(|| "ttyd-client".to_owned()),
                "higress-system",
            );
            match mask {
                0 => assert!(result.unwrap().is_none()),
                15 => assert!(result.unwrap().is_some()),
                _ => assert!(result.is_err()),
            }
        }
    }

    #[test]
    fn mtls_configuration_rejects_empty_secret_reference() {
        assert!(
            parse_ttyd_mtls(
                Some(String::new()),
                Some("ttyd-client-ca".to_owned()),
                Some("higress-system".to_owned()),
                Some("ttyd-client".to_owned()),
                "higress-system",
            )
            .is_err()
        );
    }

    #[test]
    fn mtls_namespace_must_match_the_gateway_at_startup() {
        for gateway in ["higress-system", "dedicated-gateway"] {
            assert!(
                parse_ttyd_mtls(
                    Some("ttyd-server".into()),
                    Some("ttyd-client-ca".into()),
                    Some(gateway.into()),
                    Some("client".into()),
                    gateway,
                )
                .is_ok()
            );
            let error = parse_ttyd_mtls(
                Some("ttyd-server".into()),
                Some("ttyd-client-ca".into()),
                Some("another-namespace".into()),
                Some("client".into()),
                gateway,
            )
            .unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        }
        assert!(
            parse_ttyd_mtls(None, None, None, None, "dedicated-gateway")
                .unwrap()
                .is_none()
        );
    }
}
