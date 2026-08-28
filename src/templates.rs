use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{quota::Resources, workspaces::AccessMode};

pub const TEMPLATE_API_VERSION: &str = "workspace.memeloop.dev/v1";
pub const TEMPLATE_KIND: &str = "WorkspaceTemplate";

const NO_PROXY_NODE: &str = "localhost,127.0.0.1,::1,.svc,.cluster.local,10.42.0.0/16,10.43.0.0/16,100.64.0.0/10,.k3s.onetwo.website,npmmirror.com";
const NO_PROXY_RUST: &str = "localhost,127.0.0.1,::1,.svc,.cluster.local,10.42.0.0/16,10.43.0.0/16,100.64.0.0/10,.k3s.onetwo.website,rsproxy.cn,npmmirror.com";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceTemplateDocument {
    pub api_version: String,
    pub kind: String,
    pub metadata: WorkspaceTemplateMetadata,
    pub spec: WorkspaceTemplateSpec,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceTemplateMetadata {
    pub name: String,
}

/// The complete, immutable-at-workspace-creation template snapshot.
///
/// Everything needed to render a workspace Pod is declared by the selected template and copied
/// into the workspace record. There is no second discriminator beside the template snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceTemplateSpec {
    pub image: String,
    pub access_mode: AccessMode,
    pub resources: Resources,
    pub pod_requests: PodResourceRequest,
    #[serde(default)]
    pub ephemeral_storage_limit_mib: Option<u64>,
    pub workspace_user: String,
    pub workspace_home: String,
    #[serde(default)]
    pub preserve_home_root: bool,
    #[serde(default)]
    pub buildkit: bool,
    #[serde(default)]
    pub cluster_access: bool,
    #[serde(default)]
    pub required_node_names: Vec<String>,
    #[serde(default)]
    pub preferred_node_names: Vec<String>,
    #[serde(default)]
    pub node_selector: BTreeMap<String, String>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PodResourceRequest {
    pub cpu_millis: u64,
    pub memory_mib: u64,
    #[serde(default)]
    pub ephemeral_storage_mib: Option<u64>,
}

impl WorkspaceTemplateDocument {
    pub fn parse(yaml: &str) -> Result<Self, TemplateError> {
        let document: Self = serde_yaml_ng::from_str(yaml).map_err(|_| TemplateError::Yaml)?;
        document.validate()?;
        Ok(document)
    }

    pub fn new(name: impl Into<String>, spec: WorkspaceTemplateSpec) -> Self {
        Self {
            api_version: TEMPLATE_API_VERSION.to_owned(),
            kind: TEMPLATE_KIND.to_owned(),
            metadata: WorkspaceTemplateMetadata { name: name.into() },
            spec,
        }
    }

    pub fn to_yaml(&self) -> Result<String, TemplateError> {
        self.validate()?;
        serde_yaml_ng::to_string(self).map_err(|_| TemplateError::Yaml)
    }

    pub fn validate(&self) -> Result<(), TemplateError> {
        if self.api_version != TEMPLATE_API_VERSION || self.kind != TEMPLATE_KIND {
            return Err(TemplateError::Header);
        }
        let name = self.metadata.name.trim();
        if name.is_empty()
            || name.len() > 120
            || name != self.metadata.name
            || name.chars().any(char::is_control)
        {
            return Err(TemplateError::Name);
        }
        self.spec.validate()
    }
}

impl WorkspaceTemplateSpec {
    pub fn validate(&self) -> Result<(), TemplateError> {
        if self.image.trim().is_empty()
            || self.image != self.image.trim()
            || self.image.len() > 512
            || self.image.chars().any(char::is_whitespace)
            || !self.resources.valid_workspace_request()
            || self.pod_requests.cpu_millis == 0
            || self.pod_requests.memory_mib == 0
            || self.pod_requests.cpu_millis > self.resources.cpu_millis
            || self.pod_requests.memory_mib > self.resources.memory_mib
            || self
                .pod_requests
                .ephemeral_storage_mib
                .zip(self.ephemeral_storage_limit_mib)
                .is_some_and(|(request, limit)| request > limit)
            || !valid_workspace_user(&self.workspace_user)
            || !valid_workspace_home(&self.workspace_home)
        {
            return Err(TemplateError::Spec);
        }
        if self
            .required_node_names
            .iter()
            .chain(&self.preferred_node_names)
            .any(|name| !valid_selector_part(name))
            || self
                .node_selector
                .iter()
                .any(|(key, value)| !valid_selector_part(key) || !valid_selector_part(value))
            || self
                .environment
                .iter()
                .any(|(key, value)| !valid_env_name(key) || !valid_environment_value(value))
        {
            return Err(TemplateError::Spec);
        }
        Ok(())
    }

    pub fn standard(
        image: impl Into<String>,
        access_mode: AccessMode,
        resources: Resources,
    ) -> Self {
        Self {
            image: image.into(),
            access_mode,
            resources,
            pod_requests: PodResourceRequest {
                cpu_millis: resources.cpu_millis,
                memory_mib: resources.memory_mib,
                ephemeral_storage_mib: None,
            },
            ephemeral_storage_limit_mib: None,
            workspace_user: "workspace".to_owned(),
            workspace_home: "/workspace".to_owned(),
            preserve_home_root: false,
            buildkit: false,
            cluster_access: false,
            required_node_names: Vec::new(),
            preferred_node_names: Vec::new(),
            node_selector: BTreeMap::new(),
            environment: BTreeMap::new(),
        }
    }

    /// Converts the pre-v10 database discriminator exactly once during migration.
    pub(crate) fn from_legacy(
        legacy_value: &str,
        image: impl Into<String>,
        access_mode: AccessMode,
        resources: Resources,
    ) -> Result<Self, TemplateError> {
        let mut spec = Self::standard(image, access_mode, resources);
        match legacy_value {
            "standard" => {}
            "node_dev" | "coder_node_dev" => {
                spec.workspace_user = "node-dev".to_owned();
                spec.workspace_home = "/home/node-dev".to_owned();
                spec.preserve_home_root = true;
                spec.buildkit = true;
                spec.pod_requests = PodResourceRequest {
                    cpu_millis: 1_000.min(resources.cpu_millis),
                    memory_mib: 1_024.min(resources.memory_mib),
                    ephemeral_storage_mib: Some(256),
                };
                spec.ephemeral_storage_limit_mib = Some(1_024);
                spec.required_node_names = vec!["westlake".to_owned(), "haixia".to_owned()];
                spec.environment =
                    development_environment(&spec.workspace_home, NO_PROXY_NODE, false);
            }
            "rust_dev" | "coder_rust_dev" | "coder_token_center_rust_dev" => {
                spec.workspace_user = "rust-dev".to_owned();
                spec.workspace_home = "/home/rust-dev".to_owned();
                spec.preserve_home_root = true;
                spec.buildkit = true;
                spec.pod_requests = PodResourceRequest {
                    cpu_millis: 2_000.min(resources.cpu_millis),
                    memory_mib: 4_096.min(resources.memory_mib),
                    ephemeral_storage_mib: Some(256),
                };
                spec.ephemeral_storage_limit_mib = Some(1_024);
                spec.required_node_names = vec!["westlake".to_owned(), "haixia".to_owned()];
                spec.environment =
                    development_environment(&spec.workspace_home, NO_PROXY_RUST, true);
                spec.environment
                    .insert("RUSTUP_HOME".to_owned(), "/usr/local/rustup".to_owned());
                spec.environment.insert(
                    "RUSTUP_DIST_SERVER".to_owned(),
                    "https://rsproxy.cn".to_owned(),
                );
                spec.environment.insert(
                    "RUSTUP_UPDATE_ROOT".to_owned(),
                    "https://rsproxy.cn/rustup".to_owned(),
                );
            }
            "maintainance" | "coder_cluster_admin" => {
                // Existing Coder PVCs were created for this image user and home path. The
                // template name/permission is "maintainance"; retaining the filesystem identity
                // during the v9 migration prevents an unsafe home-directory/PVC remount.
                spec.workspace_user = "cluster-admin".to_owned();
                spec.workspace_home = "/home/cluster-admin".to_owned();
                spec.preserve_home_root = true;
                spec.cluster_access = true;
                spec.pod_requests.cpu_millis = 100.min(resources.cpu_millis);
                spec.preferred_node_names = vec!["westlake".to_owned()];
                spec.node_selector
                    .insert("k3s-worker-ready".to_owned(), "true".to_owned());
                spec.environment
                    .insert("HOME".to_owned(), spec.workspace_home.clone());
                spec.environment.insert(
                    "KUBECONFIG".to_owned(),
                    format!("{}/.mwc/kubeconfig", spec.workspace_home),
                );
                spec.environment.insert("PATH".to_owned(), format!("/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin:{}/.local/bin:{}/.local/share/pnpm", spec.workspace_home, spec.workspace_home));
            }
            _ => return Err(TemplateError::Legacy),
        }
        spec.validate()?;
        Ok(spec)
    }
}

fn development_environment(home: &str, no_proxy: &str, rust: bool) -> BTreeMap<String, String> {
    let temporary_directory = if rust {
        "/tmp".to_owned()
    } else {
        format!("{home}/.tmp/dev")
    };
    let mut values = BTreeMap::from([
        ("HOME".to_owned(), home.to_owned()),
        ("NO_PROXY".to_owned(), no_proxy.to_owned()),
        ("no_proxy".to_owned(), no_proxy.to_owned()),
        ("TMPDIR".to_owned(), temporary_directory.clone()),
        ("TMP".to_owned(), temporary_directory.clone()),
        ("TEMP".to_owned(), temporary_directory),
        ("XDG_CACHE_HOME".to_owned(), format!("{home}/.cache")),
        ("XDG_CONFIG_HOME".to_owned(), format!("{home}/.config")),
        ("XDG_DATA_HOME".to_owned(), format!("{home}/.local/share")),
        ("XDG_STATE_HOME".to_owned(), format!("{home}/.local/state")),
        (
            "PLAYWRIGHT_BROWSERS_PATH".to_owned(),
            format!("{home}/.cache/ms-playwright"),
        ),
        ("DOCKER_CONFIG".to_owned(), format!("{home}/.config/docker")),
        ("NPM_CONFIG_CACHE".to_owned(), format!("{home}/.cache/npm")),
        ("PNPM_HOME".to_owned(), format!("{home}/.local/share/pnpm")),
        (
            "YARN_CACHE_FOLDER".to_owned(),
            format!("{home}/.cache/yarn"),
        ),
        (
            "BUILDKIT_HOST".to_owned(),
            format!("unix://{home}/.cache/buildkit/runtime/buildkit/buildkitd.sock"),
        ),
    ]);
    let path = if rust {
        format!(
            "/usr/local/bin:/usr/local/sbin:/usr/local/cargo/bin:{home}/.local/bin:{home}/.local/share/pnpm:{home}/.cargo/bin:/usr/sbin:/usr/bin:/sbin:/bin"
        )
    } else {
        format!(
            "/usr/local/bin:/usr/local/sbin:{home}/.local/bin:{home}/.local/share/pnpm:/usr/sbin:/usr/bin:/sbin:/bin"
        )
    };
    values.insert("PATH".to_owned(), path);
    if rust {
        values.extend([
            ("CARGO_HOME".to_owned(), format!("{home}/.cargo")),
            (
                "CARGO_TARGET_DIR".to_owned(),
                format!("{home}/.cache/cargo-target"),
            ),
            ("CARGO_HTTP_MULTIPLEXING".to_owned(), "false".to_owned()),
            (
                "CARGO_REGISTRIES_CRATES_IO_PROTOCOL".to_owned(),
                "sparse".to_owned(),
            ),
        ]);
    }
    values
}

fn valid_env_name(value: &str) -> bool {
    let mut characters = value.chars();
    matches!(characters.next(), Some(first) if first == '_' || first.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn valid_workspace_user(value: &str) -> bool {
    let mut characters = value.chars();
    value.len() <= 32
        && matches!(characters.next(), Some(first) if first == '_' || first.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '_' | '-')
        })
}

fn valid_workspace_home(value: &str) -> bool {
    value.starts_with('/')
        && value.len() <= 256
        && value != "/"
        && !value.ends_with('/')
        && !value.chars().any(char::is_whitespace)
        && value
            .split('/')
            .skip(1)
            .all(|part| !part.is_empty() && !matches!(part, "." | ".."))
}

fn valid_selector_part(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && !value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
}

fn valid_environment_value(value: &str) -> bool {
    value.len() <= 4_096
        && !value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateError {
    Yaml,
    Header,
    Name,
    Spec,
    Legacy,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_round_trip_contains_only_explicit_template_fields() {
        let document = WorkspaceTemplateDocument::new(
            "Node.js 开发",
            WorkspaceTemplateSpec::from_legacy(
                "node_dev",
                "registry.example/node@sha256:test",
                AccessMode::Internal,
                Resources {
                    cpu_millis: 6_000,
                    memory_mib: 4_096,
                    gpu_count: 0,
                    disk_gib: 60,
                },
            )
            .unwrap(),
        );
        let yaml = document.to_yaml().unwrap();
        assert!(!yaml.contains("runtimeProfile"));
        assert!(!yaml.contains("runtime_profile"));
        assert!(yaml.contains("access_mode: internal"));
        assert!(yaml.contains("workspace_user: node-dev"));
        assert_eq!(WorkspaceTemplateDocument::parse(&yaml).unwrap(), document);
        let json = serde_json::to_value(&document.spec).unwrap();
        assert_eq!(json["access_mode"], "internal");
        assert_eq!(json["workspace_user"], "node-dev");
        assert!(json.get("accessMode").is_none());
    }

    #[test]
    fn rejects_requests_above_limits() {
        let mut spec = WorkspaceTemplateSpec::standard(
            "registry.example/dev:latest",
            AccessMode::Internal,
            Resources {
                cpu_millis: 1_000,
                memory_mib: 1_024,
                gpu_count: 0,
                disk_gib: 20,
            },
        );
        spec.pod_requests.cpu_millis = 1_001;
        assert_eq!(spec.validate(), Err(TemplateError::Spec));
    }

    #[test]
    fn rejects_values_that_could_escape_generated_ssh_configuration() {
        let resources = Resources {
            cpu_millis: 1_000,
            memory_mib: 1_024,
            gpu_count: 0,
            disk_gib: 20,
        };
        let mut spec = WorkspaceTemplateSpec::standard(
            "registry.example/dev:latest",
            AccessMode::Internal,
            resources,
        );
        spec.workspace_user = "workspace\nPermitRootLogin yes".to_owned();
        assert_eq!(spec.validate(), Err(TemplateError::Spec));

        let mut spec = WorkspaceTemplateSpec::standard(
            "registry.example/dev:latest",
            AccessMode::Internal,
            resources,
        );
        spec.environment.insert(
            "HOME".to_owned(),
            "/workspace\nPermitRootLogin=yes".to_owned(),
        );
        assert_eq!(spec.validate(), Err(TemplateError::Spec));
    }
}
