use super::*;

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
