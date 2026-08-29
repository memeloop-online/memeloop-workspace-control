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
    pub environment_targets: BTreeSet<String>,
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

#[derive(Debug, Serialize)]
struct EnvironmentManifest {
    version: u8,
    environment: Vec<EnvironmentManifestEntry>,
}

#[derive(Debug, Serialize)]
struct EnvironmentManifestEntry {
    name: String,
    source: InjectionScope,
    object: &'static str,
    data_key: String,
}

#[derive(Default)]
struct MaterializedData {
    secret_environment: BTreeMap<String, ByteString>,
    config_environment: BTreeMap<String, String>,
    secret_files: BTreeMap<String, ByteString>,
    config_files: BTreeMap<String, String>,
    config_binary_files: BTreeMap<String, ByteString>,
    targets: BTreeSet<String>,
    environment_targets: BTreeSet<String>,
    files: Vec<FileManifestEntry>,
    environment: Vec<EnvironmentManifestEntry>,
}

impl MaterializedData {
    fn insert(
        &mut self,
        index: usize,
        resolved: &ResolvedInjection,
    ) -> Result<(), MaterializationError> {
        let item = &resolved.item;
        if !self.targets.insert(item.target.clone()) {
            return Err(MaterializationError::DuplicateTarget(item.target.clone()));
        }
        let value = decode_value(&item.value)?;
        match item.kind {
            InjectionKind::EnvironmentVariable => self.insert_environment(index, resolved, value),
            InjectionKind::SecretFile | InjectionKind::ConfigFile | InjectionKind::SshPublicKey => {
                self.insert_file(index, resolved, value)
            }
        }
    }

    fn insert_environment(
        &mut self,
        index: usize,
        resolved: &ResolvedInjection,
        value: Vec<u8>,
    ) -> Result<(), MaterializationError> {
        let item = &resolved.item;
        validate_environment_name(&item.target)?;
        let text = String::from_utf8(value.clone())
            .map_err(|_| MaterializationError::EnvironmentMustBeUtf8)?;
        if text.chars().any(char::is_control) {
            return Err(MaterializationError::EnvironmentContainsControl);
        }
        let data_key = format!("env-{index:04}");
        if item.sensitive {
            self.secret_environment
                .insert(item.target.clone(), ByteString(value.clone()));
            self.secret_files
                .insert(data_key.clone(), ByteString(value));
        } else {
            self.config_environment
                .insert(item.target.clone(), text.clone());
            self.config_files.insert(data_key.clone(), text);
        }
        self.environment_targets.insert(item.target.clone());
        self.environment.push(EnvironmentManifestEntry {
            name: item.target.clone(),
            source: resolved.source,
            object: if item.sensitive {
                "secret"
            } else {
                "config_map"
            },
            data_key,
        });
        Ok(())
    }

    fn insert_file(
        &mut self,
        index: usize,
        resolved: &ResolvedInjection,
        value: Vec<u8>,
    ) -> Result<(), MaterializationError> {
        let item = &resolved.item;
        validate_absolute_path(&item.target)?;
        let data_key = format!("file-{index:04}");
        let use_secret = item.sensitive || item.kind == InjectionKind::SecretFile;
        if use_secret {
            self.secret_files
                .insert(data_key.clone(), ByteString(value));
        } else if let Ok(text) = String::from_utf8(value.clone()) {
            self.config_files.insert(data_key.clone(), text);
        } else {
            self.config_binary_files
                .insert(data_key.clone(), ByteString(value));
        }
        self.files.push(FileManifestEntry {
            key: item.key.clone(),
            kind: item.kind,
            target: item.target.clone(),
            source: resolved.source,
            object: if use_secret { "secret" } else { "config_map" },
            data_key,
            mode: item.file_mode,
            owner: item.owner.clone(),
            group: item.group.clone(),
        });
        Ok(())
    }
}

pub(super) fn build(
    namespace: &str,
    labels: &BTreeMap<String, String>,
    resolved: &[ResolvedInjection],
) -> Result<InjectionMaterialization, MaterializationError> {
    let mut data = MaterializedData::default();
    for (index, item) in resolved.iter().enumerate() {
        data.insert(index, item)?;
    }
    data.config_files.insert(
        "workspace-files.json".to_owned(),
        serde_json::to_string(&FileManifest {
            version: 1,
            files: data.files,
        })?,
    );
    data.config_files.insert(
        "workspace-environment.json".to_owned(),
        serde_json::to_string(&EnvironmentManifest {
            version: 1,
            environment: data.environment,
        })?,
    );

    Ok(InjectionMaterialization {
        environment_secret: Secret {
            metadata: namespaced_metadata("workspace-environment-secret", namespace, labels),
            data: Some(data.secret_environment),
            type_: Some("Opaque".to_owned()),
            ..Secret::default()
        },
        environment_config_map: ConfigMap {
            metadata: namespaced_metadata("workspace-environment-config", namespace, labels),
            data: Some(data.config_environment),
            ..ConfigMap::default()
        },
        file_secret: Secret {
            metadata: namespaced_metadata("workspace-files-secret", namespace, labels),
            data: Some(data.secret_files),
            type_: Some("Opaque".to_owned()),
            ..Secret::default()
        },
        file_config_map: ConfigMap {
            metadata: namespaced_metadata("workspace-files-config", namespace, labels),
            data: Some(data.config_files),
            binary_data: Some(data.config_binary_files),
            ..ConfigMap::default()
        },
        environment_targets: data.environment_targets,
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
    #[error("environment variables must be UTF-8")]
    EnvironmentMustBeUtf8,
    #[error("environment variables must not contain control characters")]
    EnvironmentContainsControl,
    #[error("invalid environment variable name {0}")]
    InvalidEnvironmentName(String),
    #[error("injection target path must be absolute without parent traversal: {0}")]
    InvalidTargetPath(String),
    #[error("multiple resolved injections target {0}")]
    DuplicateTarget(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::injections::{InjectionItem, InjectionValue};

    fn resolved_environment(value: InjectionValue, sensitive: bool) -> ResolvedInjection {
        ResolvedInjection {
            source: InjectionScope::User,
            item: InjectionItem {
                key: "registry credential".to_owned(),
                kind: InjectionKind::EnvironmentVariable,
                target: "REGISTRY_TOKEN".to_owned(),
                value,
                sensitive,
                locked: false,
                version: 1,
                file_mode: None,
                owner: None,
                group: None,
                template_selector: None,
                labels: BTreeMap::new(),
            },
        }
    }

    #[test]
    fn environment_manifest_references_projected_data_without_plaintext() {
        let secret = "not-in-the-manifest";
        let materialized = build(
            "workspace-test",
            &BTreeMap::new(),
            &[resolved_environment(
                InjectionValue::Utf8(secret.to_owned()),
                true,
            )],
        )
        .unwrap();

        assert_eq!(
            materialized.environment_secret.data.as_ref().unwrap()["REGISTRY_TOKEN"].0,
            secret.as_bytes()
        );
        assert_eq!(
            materialized.file_secret.data.as_ref().unwrap()["env-0000"].0,
            secret.as_bytes()
        );
        let manifest =
            &materialized.file_config_map.data.as_ref().unwrap()["workspace-environment.json"];
        assert!(manifest.contains("REGISTRY_TOKEN"));
        assert!(manifest.contains("env-0000"));
        assert!(!manifest.contains(secret));
        assert_eq!(
            materialized.environment_targets,
            BTreeSet::from(["REGISTRY_TOKEN".to_owned()])
        );
    }

    #[test]
    fn environment_values_reject_every_control_character() {
        for value in ["line\nfeed", "carriage\rreturn", "tab\tvalue", "nul\0value"] {
            let result = build(
                "workspace-test",
                &BTreeMap::new(),
                &[resolved_environment(
                    InjectionValue::Utf8(value.to_owned()),
                    false,
                )],
            );
            assert!(matches!(
                result,
                Err(MaterializationError::EnvironmentContainsControl)
            ));
        }
    }

    #[test]
    fn explicit_sensitive_file_mode_is_preserved_in_manifest() {
        let resolved = ResolvedInjection {
            source: InjectionScope::Workspace,
            item: InjectionItem {
                key: "shared credential".to_owned(),
                kind: InjectionKind::SecretFile,
                target: "/workspace/.config/tool/credentials".to_owned(),
                value: InjectionValue::Utf8("secret".to_owned()),
                sensitive: true,
                locked: false,
                version: 1,
                file_mode: Some(0o640),
                owner: None,
                group: None,
                template_selector: None,
                labels: BTreeMap::new(),
            },
        };
        let materialized = build("workspace-test", &BTreeMap::new(), &[resolved]).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(
            &materialized.file_config_map.data.as_ref().unwrap()["workspace-files.json"],
        )
        .unwrap();

        assert_eq!(manifest["files"][0]["mode"], 0o640);
        assert!(manifest["files"][0]["owner"].is_null());
        assert!(manifest["files"][0]["group"].is_null());
    }
}
