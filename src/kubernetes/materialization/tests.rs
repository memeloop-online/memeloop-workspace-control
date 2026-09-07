use super::*;
use crate::injections::{InjectionItem, InjectionValue};
use crate::workspace_runtime::WorkspaceRuntimeNames;

fn runtime() -> WorkspaceRuntimeNames {
    WorkspaceRuntimeNames {
        namespace: "workspace-test".to_owned(),
        resource_prefix: "w-test".to_owned(),
        route_key: "test-test".to_owned(),
        resources: crate::workspace_runtime::WorkspaceResourceNames::for_prefix("w-test"),
    }
}

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
        &runtime(),
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
            &runtime(),
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
    let materialized = build(&runtime(), &BTreeMap::new(), &[resolved]).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(
        &materialized.file_config_map.data.as_ref().unwrap()["workspace-files.json"],
    )
    .unwrap();

    assert_eq!(manifest["files"][0]["mode"], 0o640);
    assert!(manifest["files"][0]["owner"].is_null());
    assert!(manifest["files"][0]["group"].is_null());
}
