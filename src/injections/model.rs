use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InjectionScope {
    Organization,
    User,
    Workspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InjectionKind {
    EnvironmentVariable,
    SecretFile,
    ConfigFile,
    SshPublicKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "encoding", content = "value", rename_all = "snake_case")]
pub enum InjectionValue {
    Utf8(String),
    Base64(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct InjectionItem {
    pub key: String,
    pub kind: InjectionKind,
    pub target: String,
    pub value: InjectionValue,
    pub sensitive: bool,
    pub locked: bool,
    pub version: u64,
    pub file_mode: Option<u32>,
    pub owner: Option<String>,
    pub group: Option<String>,
    pub template_selector: Option<String>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedInjection {
    pub scope: InjectionScope,
    pub item: InjectionItem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ResolvedInjectionSummary {
    pub key: String,
    pub kind: InjectionKind,
    pub target: String,
    pub source: InjectionScope,
    pub sensitive: bool,
    pub locked: bool,
    pub version: u64,
    pub file_mode: Option<u32>,
    pub owner: Option<String>,
    pub group: Option<String>,
    pub template_selector: Option<String>,
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedInjection {
    pub source: InjectionScope,
    pub item: InjectionItem,
}

impl ResolvedInjection {
    pub fn summary(&self) -> ResolvedInjectionSummary {
        ResolvedInjectionSummary {
            key: self.item.key.clone(),
            kind: self.item.kind,
            target: self.item.target.clone(),
            source: self.source,
            sensitive: self.item.sensitive,
            locked: self.item.locked,
            version: self.item.version,
            file_mode: self.item.file_mode,
            owner: self.item.owner.clone(),
            group: self.item.group.clone(),
            template_selector: self.item.template_selector.clone(),
            labels: self.item.labels.clone(),
        }
    }
}

impl InjectionScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Organization => "organization",
            Self::User => "user",
            Self::Workspace => "workspace",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "organization" => Some(Self::Organization),
            "user" => Some(Self::User),
            "workspace" => Some(Self::Workspace),
            _ => None,
        }
    }
}

impl InjectionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EnvironmentVariable => "environment_variable",
            Self::SecretFile => "secret_file",
            Self::ConfigFile => "config_file",
            Self::SshPublicKey => "ssh_public_key",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "environment_variable" => Some(Self::EnvironmentVariable),
            "secret_file" => Some(Self::SecretFile),
            "config_file" => Some(Self::ConfigFile),
            "ssh_public_key" => Some(Self::SshPublicKey),
            _ => None,
        }
    }
}
