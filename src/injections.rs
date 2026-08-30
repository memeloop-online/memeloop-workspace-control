mod cascade;
mod error;
mod model;
mod selection;
mod validation;

pub use cascade::resolve_injections;
pub use error::InjectionError;
pub use model::{
    InjectionItem, InjectionKind, InjectionScope, InjectionValue, ResolvedInjection,
    ResolvedInjectionSummary, ScopedInjection,
};
pub use selection::{InjectionSelection, filter_injection_refs, select_injections};
pub use validation::validate_injection_item;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use base64::Engine;
    use uuid::Uuid;

    use super::*;
    use crate::workspaces::AccessMode;

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
                kind: InjectionKind::ConfigFile,
                target: "/workspace/policy".to_owned(),
                attempted_scope: InjectionScope::User,
            }
        );
    }

    #[test]
    fn destination_overrides_do_not_depend_on_display_key() {
        let mut organization = item("组织默认值", "org", false);
        organization.kind = InjectionKind::EnvironmentVariable;
        organization.target = "HTTP_PROXY".to_owned();
        let mut workspace = item("实例代理", "workspace", false);
        workspace.kind = InjectionKind::EnvironmentVariable;
        workspace.target = "HTTP_PROXY".to_owned();

        let resolved = resolve_injections(&[organization], &[], &[workspace]).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].item.key, "实例代理");
        assert_eq!(resolved[0].source, InjectionScope::Workspace);
    }

    #[test]
    fn all_file_kinds_share_the_same_path_destination() {
        let mut organization = item("公开配置", "org", false);
        organization.kind = InjectionKind::ConfigFile;
        organization.target = "/workspace/.config/tool.conf".to_owned();
        let mut workspace = item("敏感配置", "workspace", false);
        workspace.kind = InjectionKind::SecretFile;
        workspace.target = "/workspace/.config/tool.conf".to_owned();
        workspace.sensitive = true;

        let resolved = resolve_injections(&[organization], &[], &[workspace]).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].item.kind, InjectionKind::SecretFile);
        assert_eq!(resolved[0].item.key, "敏感配置");
        assert_eq!(resolved[0].source, InjectionScope::Workspace);
    }

    #[test]
    fn locked_destination_rejects_a_differently_named_override() {
        let mut organization = item("组织代理", "org", true);
        organization.kind = InjectionKind::EnvironmentVariable;
        organization.target = "HTTP_PROXY".to_owned();
        let mut user = item("个人代理", "user", false);
        user.kind = InjectionKind::EnvironmentVariable;
        user.target = "HTTP_PROXY".to_owned();

        assert_eq!(
            resolve_injections(&[organization], &[user], &[]),
            Err(InjectionError::LockedOverride {
                key: "组织代理".to_owned(),
                kind: InjectionKind::EnvironmentVariable,
                target: "HTTP_PROXY".to_owned(),
                attempted_scope: InjectionScope::User,
            })
        );
    }

    #[test]
    fn one_scope_cannot_define_the_same_destination_twice() {
        let mut first = item("first", "one", false);
        first.target = "/workspace/shared".to_owned();
        let mut second = item("second", "two", false);
        second.target = "/workspace/shared".to_owned();

        assert_eq!(
            resolve_injections(&[], &[first, second], &[]),
            Err(InjectionError::DuplicateDestinationInScope {
                kind: InjectionKind::ConfigFile,
                target: "/workspace/shared".to_owned(),
                scope: InjectionScope::User,
            })
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
    fn environment_values_reject_binary_and_configuration_control_characters() {
        for value in ["line\nfeed", "carriage\rreturn", "tab\tvalue", "nul\0value"] {
            let mut environment = item("environment", value, false);
            environment.kind = InjectionKind::EnvironmentVariable;
            environment.target = "TOOL_VALUE".to_owned();
            assert_eq!(
                validate_injection_item(&environment),
                Err(InjectionError::InvalidEnvironmentValue)
            );
        }

        let mut binary = item("binary", "unused", false);
        binary.kind = InjectionKind::EnvironmentVariable;
        binary.target = "TOOL_VALUE".to_owned();
        binary.value =
            InjectionValue::Base64(base64::engine::general_purpose::STANDARD.encode([255]));
        assert_eq!(
            validate_injection_item(&binary),
            Err(InjectionError::InvalidEnvironmentValue)
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
