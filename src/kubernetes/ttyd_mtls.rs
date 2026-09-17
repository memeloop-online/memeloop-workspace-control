use thiserror::Error;

/// Operator-provisioned mutual TLS material for the ttyd upstream.
///
/// `server_tls_secret_name` is mounted only into ttyd and must provide `tls.crt`
/// and `tls.key`. `client_ca_secret_name` must provide `ca.crt`; separating it
/// from the cert-manager-managed server Secret permits native certificate renewal.
/// The Higress client private key stays in its configured Secret and must never be
/// copied into a workspace Pod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TtydMtlsConfig {
    pub server_tls_secret_name: String,
    pub client_ca_secret_name: String,
    pub higress_client_secret_namespace: String,
    pub higress_client_secret_name: String,
}

impl TtydMtlsConfig {
    pub fn new(
        server_tls_secret_name: String,
        client_ca_secret_name: String,
        higress_client_secret_namespace: String,
        higress_client_secret_name: String,
    ) -> Result<Self, TtydMtlsConfigError> {
        let config = Self {
            server_tls_secret_name,
            client_ca_secret_name,
            higress_client_secret_namespace,
            higress_client_secret_name,
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), TtydMtlsConfigError> {
        for value in [
            &self.server_tls_secret_name,
            &self.client_ca_secret_name,
            &self.higress_client_secret_name,
        ] {
            if !valid_dns_subdomain(value) {
                return Err(TtydMtlsConfigError::InvalidSecretReference);
            }
        }
        if !valid_dns_label(&self.higress_client_secret_namespace) {
            return Err(TtydMtlsConfigError::InvalidClientSecretNamespace);
        }
        Ok(())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TtydMtlsConfigError {
    #[error("ttyd mTLS Secret references must be lower-case DNS subdomains")]
    InvalidSecretReference,
    #[error("ttyd mTLS Higress client Secret namespace must be a lower-case DNS label")]
    InvalidClientSecretNamespace,
}

fn valid_dns_subdomain(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && value.split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 63
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                && part
                    .bytes()
                    .next()
                    .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
                && part
                    .bytes()
                    .last()
                    .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

pub(crate) fn valid_dns_label(value: &str) -> bool {
    value.len() <= 63
        && !value.is_empty()
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value.bytes().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == b'-'
        })
}
