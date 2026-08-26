use std::collections::{BTreeMap, BTreeSet};

use base64::{Engine, engine::general_purpose::STANDARD};
use k8s_openapi::{
    ByteString,
    api::core::v1::{ConfigMap, Secret},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::injections::{
    InjectionKind, InjectionScope, InjectionValue, ResolvedInjection, ResolvedInjectionSummary,
};

use super::namespaced_metadata;

#[derive(Debug)]
pub struct InjectionMaterialization {
    pub environment_secret: Secret,
    pub environment_config_map: ConfigMap,
    pub file_secret: Secret,
    pub file_config_map: ConfigMap,
    pub provenance: Vec<ResolvedInjectionSummary>,
}

impl InjectionMaterialization {
    pub fn revision(&self) -> Result<String, MaterializationError> {
        let summaries = serde_json::to_vec(&self.provenance)?;
        Ok(format!("{:x}", Sha256::digest(summaries)))
    }
}

#[derive(Debug, Serialize)]
struct FileManifest {
    version: u8,
    files: Vec<FileManifestEntry>,
}

#[derive(Debug, Serialize)]
struct FileManifestEntry {
    key: String,
    kind: InjectionKind,
    target: String,
    source: InjectionScope,
    object: &'static str,
    data_key: String,
    mode: Option<u32>,
    owner: Option<String>,
    group: Option<String>,
}

pub(super) fn build(
    namespace: &str,
    labels: &BTreeMap<String, String>,
    resolved: &[ResolvedInjection],
) -> Result<InjectionMaterialization, MaterializationError> {
    let mut secret_environment = BTreeMap::<String, ByteString>::new();
    let mut config_environment = BTreeMap::<String, String>::new();
    let mut secret_files = BTreeMap::<String, ByteString>::new();
    let mut config_files = BTreeMap::<String, String>::new();
    let mut config_binary_files = BTreeMap::<String, ByteString>::new();
    let mut targets = BTreeSet::<String>::new();
    let mut files = Vec::new();

    for (index, resolved_item) in resolved.iter().enumerate() {
        let item = &resolved_item.item;
        if !targets.insert(item.target.clone()) {
            return Err(MaterializationError::DuplicateTarget(item.target.clone()));
        }
        let value = decode_value(&item.value)?;
        match item.kind {
            InjectionKind::EnvironmentVariable => {
                validate_environment_name(&item.target)?;
                if item.sensitive {
                    secret_environment.insert(item.target.clone(), ByteString(value));
                } else {
                    let text = String::from_utf8(value)
                        .map_err(|_| MaterializationError::ConfigEnvironmentMustBeUtf8)?;
                    config_environment.insert(item.target.clone(), text);
                }
            }
            InjectionKind::SecretFile | InjectionKind::ConfigFile | InjectionKind::SshPublicKey => {
                validate_absolute_path(&item.target)?;
                let data_key = format!("file-{index:04}");
                let use_secret = item.sensitive || item.kind == InjectionKind::SecretFile;
                if use_secret {
                    secret_files.insert(data_key.clone(), ByteString(value));
                } else if let Ok(text) = String::from_utf8(value.clone()) {
                    config_files.insert(data_key.clone(), text);
                } else {
                    config_binary_files.insert(data_key.clone(), ByteString(value));
                }
                files.push(FileManifestEntry {
                    key: item.key.clone(),
                    kind: item.kind,
                    target: item.target.clone(),
                    source: resolved_item.source,
                    object: if use_secret { "secret" } else { "config_map" },
                    data_key,
                    mode: item.file_mode,
                    owner: item.owner.clone(),
                    group: item.group.clone(),
                });
            }
        }
    }
    config_files.insert(
        "workspace-files.json".to_owned(),
        serde_json::to_string(&FileManifest { version: 1, files })?,
    );

    Ok(InjectionMaterialization {
        environment_secret: Secret {
            metadata: namespaced_metadata("workspace-environment-secret", namespace, labels),
            data: Some(secret_environment),
            type_: Some("Opaque".to_owned()),
            ..Secret::default()
        },
        environment_config_map: ConfigMap {
            metadata: namespaced_metadata("workspace-environment-config", namespace, labels),
            data: Some(config_environment),
            ..ConfigMap::default()
        },
        file_secret: Secret {
            metadata: namespaced_metadata("workspace-files-secret", namespace, labels),
            data: Some(secret_files),
            type_: Some("Opaque".to_owned()),
            ..Secret::default()
        },
        file_config_map: ConfigMap {
            metadata: namespaced_metadata("workspace-files-config", namespace, labels),
            data: Some(config_files),
            binary_data: Some(config_binary_files),
            ..ConfigMap::default()
        },
        provenance: resolved.iter().map(ResolvedInjection::summary).collect(),
    })
}

fn decode_value(value: &InjectionValue) -> Result<Vec<u8>, MaterializationError> {
    match value {
        InjectionValue::Utf8(value) => Ok(value.as_bytes().to_vec()),
        InjectionValue::Base64(value) => STANDARD
            .decode(value)
            .map_err(|_| MaterializationError::InvalidBase64),
    }
}

fn validate_environment_name(name: &str) -> Result<(), MaterializationError> {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return Err(MaterializationError::InvalidEnvironmentName(
            name.to_owned(),
        ));
    };
    if !(first == '_' || first.is_ascii_alphabetic())
        || !characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        return Err(MaterializationError::InvalidEnvironmentName(
            name.to_owned(),
        ));
    }
    Ok(())
}

fn validate_absolute_path(path: &str) -> Result<(), MaterializationError> {
    if !path.starts_with('/') || path.split('/').any(|component| component == "..") {
        return Err(MaterializationError::InvalidTargetPath(path.to_owned()));
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum MaterializationError {
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("injection contains invalid Base64")]
    InvalidBase64,
    #[error("non-sensitive environment variables must be UTF-8")]
    ConfigEnvironmentMustBeUtf8,
    #[error("invalid environment variable name {0}")]
    InvalidEnvironmentName(String),
    #[error("injection target path must be absolute without parent traversal: {0}")]
    InvalidTargetPath(String),
    #[error("multiple resolved injections target {0}")]
    DuplicateTarget(String),
}
