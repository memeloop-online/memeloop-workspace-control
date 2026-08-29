use std::{
    collections::BTreeSet,
    fs,
    path::{Component as PathComponent, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::{ConfigurationSchema, PluginError};

const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_COMPONENT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PACKAGE_BYTES: u64 = 80 * 1024 * 1024;
const MAX_PACKAGE_FILES: usize = 64;
const SUPPORTED_INTERFACE: &str = ">=0.1.0, <0.2.0";

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub wit_version: String,
    pub wasm: Option<String>,
    pub workspace_create_policy: bool,
    pub denial_codes: Vec<String>,
    pub configuration: Option<PluginConfigurationContribution>,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginConfigurationContribution {
    pub schema: Value,
    pub default: Value,
}

pub(super) struct ValidatedPackage {
    pub manifest: PluginManifest,
    pub component_path: Option<PathBuf>,
    pub configuration_schema: Option<ConfigurationSchema>,
}

pub(super) fn discover(root: &Path) -> Result<Vec<ValidatedPackage>, PluginError> {
    if !root.is_dir() {
        return Err(PluginError::invalid("plugin directory does not exist"));
    }
    let root = fs::canonicalize(root).map_err(|_| PluginError::Package)?;
    let mut directories = Vec::new();
    for entry in fs::read_dir(&root).map_err(|_| PluginError::Package)? {
        let entry = entry.map_err(|_| PluginError::Package)?;
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let kind = entry.file_type().map_err(|_| PluginError::Package)?;
        if kind.is_symlink() || !kind.is_dir() {
            return Err(PluginError::invalid(
                "plugin directory may contain only package directories",
            ));
        }
        directories.push(entry.path());
    }
    directories.sort();

    let mut ids = BTreeSet::new();
    let mut packages = Vec::with_capacity(directories.len());
    for directory in directories {
        let package = validate_package(&root, &directory)?;
        if !ids.insert(package.manifest.id.clone()) {
            return Err(PluginError::invalid("duplicate plugin id"));
        }
        packages.push(package);
    }
    packages.sort_by(|left, right| left.manifest.id.cmp(&right.manifest.id));
    Ok(packages)
}

fn validate_package(root: &Path, directory: &Path) -> Result<ValidatedPackage, PluginError> {
    let directory = fs::canonicalize(directory).map_err(|_| PluginError::Package)?;
    if !directory.starts_with(root) {
        return Err(PluginError::invalid(
            "plugin package escapes plugin directory",
        ));
    }
    validate_package_tree(&directory)?;
    let manifest_path = directory.join("plugin.json");
    require_regular_file(&manifest_path, MAX_MANIFEST_BYTES, "manifest")?;
    let bytes = fs::read(&manifest_path).map_err(|_| PluginError::Package)?;
    let manifest: PluginManifest = serde_json::from_slice(&bytes)
        .map_err(|_| PluginError::invalid("plugin.json is invalid"))?;
    validate_manifest(&manifest)?;
    let configuration_schema = manifest
        .configuration
        .as_ref()
        .map(|configuration| {
            let schema = ConfigurationSchema::compile(&configuration.schema)?;
            schema.validate(&configuration.default)?;
            Ok::<ConfigurationSchema, PluginError>(schema)
        })
        .transpose()?;
    let component_path = manifest
        .wasm
        .as_deref()
        .map(|relative| safe_component_path(&directory, relative))
        .transpose()?;
    Ok(ValidatedPackage {
        manifest,
        component_path,
        configuration_schema,
    })
}

fn validate_manifest(manifest: &PluginManifest) -> Result<(), PluginError> {
    if manifest.id.is_empty()
        || manifest.id.len() > 64
        || !manifest
            .id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(PluginError::invalid("plugin id is invalid"));
    }
    if manifest.name.trim().is_empty()
        || manifest.name.len() > 120
        || manifest.description.len() > 2_000
        || manifest.name.chars().any(char::is_control)
        || manifest.description.chars().any(char::is_control)
    {
        return Err(PluginError::invalid("plugin metadata is invalid"));
    }
    semver::Version::parse(&manifest.version)
        .map_err(|_| PluginError::invalid("plugin version is invalid"))?;
    let interface = semver::Version::parse(&manifest.wit_version)
        .map_err(|_| PluginError::invalid("plugin interface version is invalid"))?;
    if !semver::VersionReq::parse(SUPPORTED_INTERFACE)
        .map_err(|_| PluginError::Package)?
        .matches(&interface)
    {
        return Err(PluginError::invalid(
            "plugin interface version is not supported",
        ));
    }
    if manifest.workspace_create_policy && manifest.wasm.is_none() {
        return Err(PluginError::invalid(
            "workspace create policy requires a component",
        ));
    }
    if manifest.wasm.is_some() && !manifest.workspace_create_policy {
        return Err(PluginError::invalid(
            "components must declare workspace_create_policy",
        ));
    }
    let mut unique = BTreeSet::new();
    for code in &manifest.denial_codes {
        if code.is_empty()
            || code.len() > 64
            || !code.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
            })
            || !unique.insert(code)
        {
            return Err(PluginError::invalid("plugin denial code is invalid"));
        }
    }
    Ok(())
}

fn safe_component_path(root: &Path, relative: &str) -> Result<PathBuf, PluginError> {
    let relative = Path::new(relative);
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                PathComponent::ParentDir | PathComponent::RootDir | PathComponent::Prefix(_)
            )
        })
    {
        return Err(PluginError::invalid("component path is unsafe"));
    }
    let component = fs::canonicalize(root.join(relative)).map_err(|_| PluginError::Package)?;
    if !component.starts_with(root) {
        return Err(PluginError::invalid("component path escapes package"));
    }
    require_regular_file(&component, MAX_COMPONENT_BYTES, "component")?;
    Ok(component)
}

fn validate_package_tree(root: &Path) -> Result<(), PluginError> {
    let mut pending = vec![(root.to_path_buf(), 0_usize)];
    let mut files = 0_usize;
    let mut bytes = 0_u64;
    while let Some((directory, depth)) = pending.pop() {
        if depth > 8 {
            return Err(PluginError::invalid("plugin package is too deeply nested"));
        }
        for entry in fs::read_dir(directory).map_err(|_| PluginError::Package)? {
            let entry = entry.map_err(|_| PluginError::Package)?;
            let metadata = fs::symlink_metadata(entry.path()).map_err(|_| PluginError::Package)?;
            if metadata.file_type().is_symlink() {
                return Err(PluginError::invalid(
                    "plugin packages cannot contain symlinks",
                ));
            }
            if metadata.is_dir() {
                pending.push((entry.path(), depth + 1));
            } else if metadata.is_file() {
                files = files.saturating_add(1);
                bytes = bytes.saturating_add(metadata.len());
            } else {
                return Err(PluginError::invalid(
                    "plugin package contains a special file",
                ));
            }
            if files > MAX_PACKAGE_FILES || bytes > MAX_PACKAGE_BYTES {
                return Err(PluginError::invalid("plugin package exceeds size limits"));
            }
        }
    }
    Ok(())
}

fn require_regular_file(path: &Path, maximum: u64, kind: &str) -> Result<(), PluginError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| PluginError::Package)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > maximum {
        return Err(PluginError::invalid(format!("plugin {kind} is invalid")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use super::*;

    #[test]
    fn discovery_rejects_unknown_manifest_fields() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("policy");
        fs::create_dir(&package).unwrap();
        let mut manifest = valid_manifest();
        manifest["unknown"] = json!(true);
        fs::write(package.join("plugin.json"), manifest.to_string()).unwrap();
        assert!(discover(root.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn discovery_rejects_package_symlinks() {
        let root = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/tmp", root.path().join("policy")).unwrap();
        assert!(discover(root.path()).is_err());
    }

    fn valid_manifest() -> Value {
        json!({
            "id":"policy",
            "name":"Policy",
            "version":"1.0.0",
            "description":"Example",
            "wit_version":"0.1.0",
            "wasm":null,
            "workspace_create_policy":false,
            "denial_codes":[],
            "configuration":null
        })
    }
}
