use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::workspaces::AccessMode;

mod validation;

pub use validation::validate_injection_item;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
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

#[derive(Debug, Clone, Copy)]
pub struct InjectionSelection<'a> {
    pub workspace_id: Option<Uuid>,
    pub organization_id: Uuid,
    pub owner_id: Uuid,
    pub template_id: Option<Uuid>,
    pub image: &'a str,
    pub access_mode: AccessMode,
}

pub fn select_injections(
    items: &[InjectionItem],
    selection: InjectionSelection<'_>,
) -> Vec<InjectionItem> {
    items
        .iter()
        .filter(|item| injection_matches(item, selection))
        .cloned()
        .collect()
}

pub fn filter_injection_refs(
    items: &[InjectionItem],
    refs: Option<&[String]>,
    include_locked: bool,
) -> Vec<InjectionItem> {
    let Some(refs) = refs else {
        return items.to_vec();
    };
    let selected = refs.iter().map(String::as_str).collect::<BTreeSet<_>>();
    items
        .iter()
        .filter(|item| (include_locked && item.locked) || selected.contains(item.key.as_str()))
        .cloned()
        .collect()
}

fn injection_matches(item: &InjectionItem, selection: InjectionSelection<'_>) -> bool {
    let template_matches = item.template_selector.as_deref().is_none_or(|selector| {
        selector == "*"
            || selection
                .template_id
                .is_some_and(|template_id| selector == template_id.to_string())
    });
    template_matches
        && item.labels.iter().all(|(key, expected)| {
            selection_label(selection, key).is_some_and(|actual| actual == *expected)
        })
}

fn selection_label(selection: InjectionSelection<'_>, key: &str) -> Option<String> {
    match key {
        "workspace_id" => selection.workspace_id.map(|value| value.to_string()),
        "organization_id" => Some(selection.organization_id.to_string()),
        "owner_id" => Some(selection.owner_id.to_string()),
        "template_id" => selection.template_id.map(|value| value.to_string()),
        "image" => Some(selection.image.to_owned()),
        "access_mode" => Some(selection.access_mode.as_str().to_owned()),
        _ => None,
    }
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

pub fn resolve_injections(
    organization: &[InjectionItem],
    user: &[InjectionItem],
    workspace: &[InjectionItem],
) -> Result<Vec<ResolvedInjection>, InjectionError> {
    let mut resolved = BTreeMap::<String, ResolvedInjection>::new();

    apply_scope(&mut resolved, InjectionScope::Organization, organization)?;
    apply_scope(&mut resolved, InjectionScope::User, user)?;
    apply_scope(&mut resolved, InjectionScope::Workspace, workspace)?;

    Ok(resolved.into_values().collect())
}

fn apply_scope(
    resolved: &mut BTreeMap<String, ResolvedInjection>,
    scope: InjectionScope,
    items: &[InjectionItem],
) -> Result<(), InjectionError> {
    let mut seen = BTreeMap::<&str, ()>::new();
    for item in items {
        if item.key.is_empty() {
            return Err(InjectionError::EmptyKey);
        }
        if item.locked && scope != InjectionScope::Organization {
            return Err(InjectionError::LockOutsideOrganization {
                key: item.key.clone(),
                scope,
            });
        }
        if seen.insert(&item.key, ()).is_some() {
            return Err(InjectionError::DuplicateInScope {
                key: item.key.clone(),
                scope,
            });
        }
        if let Some(existing) = resolved.get(&item.key)
            && existing.item.locked
        {
            return Err(InjectionError::LockedOverride {
                key: item.key.clone(),
                attempted_scope: scope,
            });
        }
        resolved.insert(
            item.key.clone(),
            ResolvedInjection {
                source: scope,
                item: item.clone(),
            },
        );
    }
    Ok(())
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InjectionError {
    #[error("injection key must not be empty")]
    EmptyKey,
    #[error(
        "injection key must use 1-128 letters, digits, spaces, dots, underscores, or hyphens without leading or trailing spaces"
    )]
    InvalidKey,
    #[error("injection target is invalid for its kind")]
    InvalidTarget,
    #[error("file mode must be between 000 and 777")]
    InvalidFileMode,
    #[error("owner and group must be safe local account names")]
    InvalidOwnerOrGroup,
    #[error("template selector must be a template UUID or *")]
    InvalidTemplateSelector,
    #[error("label selectors must use non-empty bounded keys and string values")]
    InvalidLabelSelector,
    #[error("Base64 injection value is invalid")]
    InvalidBase64,
    #[error("SSH public key injection must contain one valid OpenSSH public key")]
    InvalidSshPublicKey,
    #[error("duplicate injection {key} in {scope:?} scope")]
    DuplicateInScope { key: String, scope: InjectionScope },
    #[error("only organization injections may be locked: {key} in {scope:?} scope")]
    LockOutsideOrganization { key: String, scope: InjectionScope },
    #[error("{attempted_scope:?} injection cannot override locked organization injection {key}")]
    LockedOverride {
        key: String,
        attempted_scope: InjectionScope,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(key: &str, value: &str, locked: bool) -> InjectionItem {
        InjectionItem {
            key: key.to_owned(),
            kind: InjectionKind::ConfigFile,
            target: format!("/workspace/{key}"),
            value: InjectionValue::Utf8(value.to_owned()),
            sensitive: false,
            locked,
            version: 1,
            file_mode: None,
            owner: None,
            group: None,
            template_selector: None,
            labels: BTreeMap::new(),
        }
    }

    #[test]
    fn more_specific_scope_wins_and_reports_source() {
        let result = resolve_injections(
            &[item("editor", "org", false)],
            &[item("editor", "user", false)],
            &[item("editor", "workspace", false)],
        )
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source, InjectionScope::Workspace);
        assert_eq!(
            result[0].item.value,
            InjectionValue::Utf8("workspace".to_owned())
        );
    }

    #[test]
    fn organization_lock_rejects_any_override() {
        let error = resolve_injections(
            &[item("policy", "required", true)],
            &[item("policy", "changed", false)],
            &[],
        )
        .unwrap_err();
        assert_eq!(
            error,
            InjectionError::LockedOverride {
                key: "policy".to_owned(),
                attempted_scope: InjectionScope::User,
            }
        );
    }

    #[test]
    fn multiline_utf8_is_preserved_byte_for_byte() {
        let multiline = "first\n\n  indented\nlast\n";
        let result = resolve_injections(&[], &[], &[item("settings", multiline, false)]).unwrap();
        assert_eq!(
            result[0].item.value,
            InjectionValue::Utf8(multiline.to_owned())
        );
    }

    #[test]
    fn unicode_display_keys_are_valid_but_unsafe_separators_are_not() {
        let unicode = item("Windows 游戏电脑公钥", "value", false);
        validate_injection_item(&unicode).unwrap();

        let invalid = item("folder/key", "value", false);
        assert_eq!(
            validate_injection_item(&invalid),
            Err(InjectionError::InvalidKey)
        );
    }

    #[test]
    fn summaries_never_contain_values() {
        let secret = InjectionItem {
            sensitive: true,
            value: InjectionValue::Utf8("do-not-disclose".to_owned()),
            ..item("token", "ignored", false)
        };
        let resolved = resolve_injections(&[], &[], &[secret]).unwrap();
        let json = serde_json::to_string(&resolved[0].summary()).unwrap();
        assert!(!json.contains("do-not-disclose"));
    }

    #[test]
    fn template_and_standard_label_selectors_filter_items() {
        let organization_id = Uuid::now_v7();
        let owner_id = Uuid::now_v7();
        let template_id = Uuid::now_v7();
        let selection = InjectionSelection {
            workspace_id: None,
            organization_id,
            owner_id,
            template_id: Some(template_id),
            image: "registry.example/workspace:1",
            access_mode: AccessMode::Public,
        };
        let mut matching = item("matching", "yes", false);
        matching.template_selector = Some(template_id.to_string());
        matching
            .labels
            .insert("access_mode".to_owned(), "public".to_owned());
        let mut excluded = item("excluded", "no", false);
        excluded
            .labels
            .insert("unknown-label".to_owned(), "value".to_owned());

        let selected = select_injections(&[matching, excluded], selection);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].key, "matching");
    }

    #[test]
    fn explicit_references_filter_items_but_cannot_omit_locked_policy() {
        let locked = item("locked-policy", "required", true);
        let selected = item("selected", "yes", false);
        let omitted = item("omitted", "no", false);
        let available = [locked, selected, omitted];

        let filtered = filter_injection_refs(&available, Some(&["selected".to_owned()]), true);
        assert_eq!(
            filtered
                .iter()
                .map(|item| item.key.as_str())
                .collect::<Vec<_>>(),
            ["locked-policy", "selected"]
        );
        assert_eq!(
            filter_injection_refs(&available, Some(&[]), true)
                .iter()
                .map(|item| item.key.as_str())
                .collect::<Vec<_>>(),
            ["locked-policy"]
        );
        assert_eq!(filter_injection_refs(&available, None, true), available);
    }
}
