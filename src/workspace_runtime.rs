use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::config::InstallationId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceRuntimeNamingScheme {
    LegacyV1,
    PrefixedV2,
}

impl WorkspaceRuntimeNamingScheme {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LegacyV1 => "legacy_v1",
            Self::PrefixedV2 => "prefixed_v2",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "legacy_v1" => Some(Self::LegacyV1),
            "prefixed_v2" => Some(Self::PrefixedV2),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceNamespaceScope {
    Dedicated,
    Shared,
}

impl WorkspaceNamespaceScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dedicated => "dedicated",
            Self::Shared => "shared",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "dedicated" => Some(Self::Dedicated),
            "shared" => Some(Self::Shared),
            _ => None,
        }
    }
}

/// Immutable placement and naming data captured when a workspace is created.
///
/// Reconcilers must consume this value from the database. They must never derive
/// an existing workspace's namespace or object names from current deployment
/// configuration, because doing so could orphan or overwrite its PVC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct WorkspaceRuntimeIdentity {
    pub naming_scheme: WorkspaceRuntimeNamingScheme,
    pub namespace_scope: WorkspaceNamespaceScope,
    pub namespace: String,
    pub resource_prefix: String,
    pub route_key: String,
}

impl WorkspaceRuntimeIdentity {
    pub fn legacy_v1(
        installation_id: &InstallationId,
        workspace_short_id: &str,
    ) -> Result<Self, WorkspaceRuntimeIdentityError> {
        let identity = Self {
            naming_scheme: WorkspaceRuntimeNamingScheme::LegacyV1,
            namespace_scope: WorkspaceNamespaceScope::Dedicated,
            namespace: installation_id
                .workspace_namespace(workspace_short_id)
                .map_err(|_| WorkspaceRuntimeIdentityError::InvalidIdentity)?,
            resource_prefix: "workspace".to_owned(),
            route_key: workspace_short_id.to_owned(),
        };
        identity.validate()?;
        Ok(identity)
    }

    pub fn prefixed_v2(
        installation_id: &InstallationId,
        workspace_id: Uuid,
        workspace_short_id: &str,
        shared_namespace: Option<&str>,
    ) -> Result<Self, WorkspaceRuntimeIdentityError> {
        let namespace = match shared_namespace {
            Some(namespace) => namespace.to_owned(),
            None => installation_id
                .workspace_namespace(workspace_short_id)
                .map_err(|_| WorkspaceRuntimeIdentityError::InvalidIdentity)?,
        };
        let identity = Self {
            naming_scheme: WorkspaceRuntimeNamingScheme::PrefixedV2,
            namespace_scope: if shared_namespace.is_some() {
                WorkspaceNamespaceScope::Shared
            } else {
                WorkspaceNamespaceScope::Dedicated
            },
            namespace,
            resource_prefix: format!("w-{workspace_short_id}"),
            route_key: format!("{}-{workspace_short_id}", installation_id.as_str()),
        };
        identity.validate_for_workspace(installation_id, workspace_id, workspace_short_id)?;
        Ok(identity)
    }

    pub fn validate(&self) -> Result<(), WorkspaceRuntimeIdentityError> {
        valid_dns_label(&self.namespace, 63)?;
        valid_dns_label(&self.resource_prefix, 40)?;
        valid_dns_label(&self.route_key, 63)?;
        match self.naming_scheme {
            WorkspaceRuntimeNamingScheme::LegacyV1
                if self.namespace_scope != WorkspaceNamespaceScope::Dedicated
                    || self.resource_prefix != "workspace" =>
            {
                Err(WorkspaceRuntimeIdentityError::InvalidIdentity)
            }
            _ => {
                self.names().validate()?;
                Ok(())
            }
        }
    }

    pub fn validate_for_workspace(
        &self,
        installation_id: &InstallationId,
        workspace_id: Uuid,
        workspace_short_id: &str,
    ) -> Result<(), WorkspaceRuntimeIdentityError> {
        self.validate()?;
        match self.naming_scheme {
            WorkspaceRuntimeNamingScheme::LegacyV1 => {
                let expected_namespace = installation_id
                    .workspace_namespace(workspace_short_id)
                    .map_err(|_| WorkspaceRuntimeIdentityError::InvalidIdentity)?;
                if self.namespace_scope != WorkspaceNamespaceScope::Dedicated
                    || self.namespace != expected_namespace
                    || self.resource_prefix != "workspace"
                    || self.route_key != workspace_short_id
                {
                    return Err(WorkspaceRuntimeIdentityError::InvalidIdentity);
                }
            }
            WorkspaceRuntimeNamingScheme::PrefixedV2 => {
                if workspace_short_id != workspace_short_id_for(workspace_id)
                    || self.resource_prefix != format!("w-{workspace_short_id}")
                    || self.route_key
                        != format!("{}-{workspace_short_id}", installation_id.as_str())
                {
                    return Err(WorkspaceRuntimeIdentityError::InvalidIdentity);
                }
                if self.namespace_scope == WorkspaceNamespaceScope::Dedicated
                    && self.namespace
                        != installation_id
                            .workspace_namespace(workspace_short_id)
                            .map_err(|_| WorkspaceRuntimeIdentityError::InvalidIdentity)?
                {
                    return Err(WorkspaceRuntimeIdentityError::InvalidIdentity);
                }
            }
        }
        Ok(())
    }

    pub fn names(&self) -> WorkspaceResourceNames {
        match self.naming_scheme {
            WorkspaceRuntimeNamingScheme::LegacyV1 => WorkspaceResourceNames::legacy(),
            WorkspaceRuntimeNamingScheme::PrefixedV2 => {
                WorkspaceResourceNames::prefixed(&self.resource_prefix)
            }
        }
    }

    pub fn web_shell_path(&self) -> String {
        format!("/shell/{}/", self.route_key)
    }
}

/// Stable, collision-resistant suffix used by API aliases and Kubernetes names.
///
/// UUIDv7's leading bits encode time and are often shared by workspaces created
/// in one batch, so runtime names use the lower 64 random bits instead.
pub fn workspace_short_id_for(workspace_id: Uuid) -> String {
    let encoded_id = workspace_id.simple().to_string();
    encoded_id[encoded_id.len() - 16..].to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceResourceNames {
    pub stateful_set: String,
    pub service: String,
    pub ssh_service: String,
    pub service_account: String,
    pub workspace_config: String,
    pub ssh_identity_secret: String,
    pub environment_secret: String,
    pub environment_config_map: String,
    pub files_secret: String,
    pub files_config_map: String,
    pub network_policy: String,
    pub web_shell_ingress: String,
    pub data_claim_template: String,
}

impl WorkspaceResourceNames {
    fn legacy() -> Self {
        Self {
            stateful_set: "workspace".to_owned(),
            service: "workspace".to_owned(),
            ssh_service: "workspace-ssh".to_owned(),
            service_account: "workspace-admin".to_owned(),
            workspace_config: "workspace-config".to_owned(),
            ssh_identity_secret: "workspace-ssh-identity".to_owned(),
            environment_secret: "workspace-environment-secret".to_owned(),
            environment_config_map: "workspace-environment-config".to_owned(),
            files_secret: "workspace-files-secret".to_owned(),
            files_config_map: "workspace-files-config".to_owned(),
            network_policy: "workspace-ingress".to_owned(),
            web_shell_ingress: "web-shell".to_owned(),
            data_claim_template: "workspace-data".to_owned(),
        }
    }

    fn prefixed(prefix: &str) -> Self {
        let named = |suffix: &str| format!("{prefix}-{suffix}");
        Self {
            stateful_set: prefix.to_owned(),
            service: prefix.to_owned(),
            ssh_service: named("ssh"),
            service_account: named("admin"),
            workspace_config: named("config"),
            ssh_identity_secret: named("ssh-identity"),
            environment_secret: named("environment-secret"),
            environment_config_map: named("environment-config"),
            files_secret: named("files-secret"),
            files_config_map: named("files-config"),
            network_policy: named("ingress"),
            web_shell_ingress: named("web-shell"),
            data_claim_template: "workspace-data".to_owned(),
        }
    }

    pub fn pod_ordinal_zero(&self) -> String {
        format!("{}-0", self.stateful_set)
    }

    pub fn data_pvc_ordinal_zero(&self) -> String {
        format!("{}-{}-0", self.data_claim_template, self.stateful_set)
    }

    fn validate(&self) -> Result<(), WorkspaceRuntimeIdentityError> {
        for name in [
            &self.stateful_set,
            &self.service,
            &self.ssh_service,
            &self.service_account,
            &self.workspace_config,
            &self.ssh_identity_secret,
            &self.environment_secret,
            &self.environment_config_map,
            &self.files_secret,
            &self.files_config_map,
            &self.network_policy,
            &self.web_shell_ingress,
            &self.data_claim_template,
            &self.pod_ordinal_zero(),
            &self.data_pvc_ordinal_zero(),
        ] {
            valid_dns_label(name, 63)?;
        }
        Ok(())
    }
}

fn valid_dns_label(value: &str, max_length: usize) -> Result<(), WorkspaceRuntimeIdentityError> {
    let valid_edge = |character: char| character.is_ascii_lowercase() || character.is_ascii_digit();
    if value.is_empty()
        || value.len() > max_length
        || !value
            .chars()
            .all(|character| valid_edge(character) || character == '-')
        || !value.chars().next().is_some_and(valid_edge)
        || !value.chars().last().is_some_and(valid_edge)
    {
        return Err(WorkspaceRuntimeIdentityError::InvalidIdentity);
    }
    Ok(())
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkspaceRuntimeIdentityError {
    #[error("workspace runtime identity is invalid")]
    InvalidIdentity,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_names_remain_byte_for_byte_compatible() {
        let installation = "k3si-7032544955".parse().unwrap();
        let identity = WorkspaceRuntimeIdentity::legacy_v1(&installation, "01a04415").unwrap();
        let names = identity.names();
        assert_eq!(identity.namespace, "ws-k3si-7032544955-01a04415");
        assert_eq!(names.stateful_set, "workspace");
        assert_eq!(names.service, "workspace");
        assert_eq!(names.ssh_service, "workspace-ssh");
        assert_eq!(names.data_pvc_ordinal_zero(), "workspace-data-workspace-0");
        assert_eq!(identity.web_shell_path(), "/shell/01a04415/");
    }

    #[test]
    fn prefixed_names_do_not_collide_in_one_namespace() {
        let installation = "internal-a".parse().unwrap();
        let first_id = Uuid::parse_str("018f0000-0000-7000-8000-000000000001").unwrap();
        let second_id = Uuid::parse_str("018f0000-0000-7000-8000-000000000002").unwrap();
        let first = WorkspaceRuntimeIdentity::prefixed_v2(
            &installation,
            first_id,
            "8000000000000001",
            Some("ws-internal-a"),
        )
        .unwrap();
        let second = WorkspaceRuntimeIdentity::prefixed_v2(
            &installation,
            second_id,
            "8000000000000002",
            Some("ws-internal-a"),
        )
        .unwrap();
        assert_eq!(first.namespace, second.namespace);
        assert_ne!(first.names().stateful_set, second.names().stateful_set);
        assert_ne!(
            first.names().data_pvc_ordinal_zero(),
            second.names().data_pvc_ordinal_zero()
        );
        first.validate().unwrap();
        second.validate().unwrap();
    }

    #[test]
    fn prefixed_identity_rejects_a_short_id_from_another_workspace() {
        let installation = "internal-a".parse().unwrap();
        let id = Uuid::parse_str("018f0000-0000-7000-8000-000000000001").unwrap();
        assert!(
            WorkspaceRuntimeIdentity::prefixed_v2(
                &installation,
                id,
                "8000000000000002",
                Some("ws-internal-a"),
            )
            .is_err()
        );
    }

    #[test]
    fn complete_validation_rejects_tampered_legacy_identity_fields() {
        let installation = "internal-a".parse().unwrap();
        let mut identity =
            WorkspaceRuntimeIdentity::legacy_v1(&installation, "8000000000000001").unwrap();
        let workspace_id = Uuid::parse_str("018f0000-0000-7000-8000-000000000001").unwrap();

        let mutations: [fn(&mut WorkspaceRuntimeIdentity); 4] = [
            |identity: &mut WorkspaceRuntimeIdentity| {
                identity.namespace_scope = WorkspaceNamespaceScope::Shared
            },
            |identity: &mut WorkspaceRuntimeIdentity| {
                identity.namespace = "other-namespace".to_owned()
            },
            |identity: &mut WorkspaceRuntimeIdentity| identity.resource_prefix = "other".to_owned(),
            |identity: &mut WorkspaceRuntimeIdentity| identity.route_key = "other".to_owned(),
        ];
        for mutate in mutations {
            let mut tampered = identity.clone();
            mutate(&mut tampered);
            assert!(
                tampered
                    .validate_for_workspace(&installation, workspace_id, "8000000000000001")
                    .is_err()
            );
        }

        identity.naming_scheme = WorkspaceRuntimeNamingScheme::PrefixedV2;
        assert!(
            identity
                .validate_for_workspace(&installation, workspace_id, "8000000000000001")
                .is_err()
        );
    }

    #[test]
    fn complete_validation_binds_prefixed_identity_to_installation_and_workspace() {
        let installation = "internal-a".parse().unwrap();
        let other_installation = "internal-b".parse().unwrap();
        let workspace_id = Uuid::parse_str("018f0000-0000-7000-8000-000000000001").unwrap();
        let short_id = workspace_short_id_for(workspace_id);
        let identity =
            WorkspaceRuntimeIdentity::prefixed_v2(&installation, workspace_id, &short_id, None)
                .unwrap();

        assert!(
            identity
                .validate_for_workspace(&other_installation, workspace_id, &short_id)
                .is_err()
        );
        assert!(
            identity
                .validate_for_workspace(&installation, workspace_id, "8000000000000002")
                .is_err()
        );
        let mutations: [fn(&mut WorkspaceRuntimeIdentity); 3] = [
            |identity: &mut WorkspaceRuntimeIdentity| {
                identity.namespace = "other-namespace".to_owned()
            },
            |identity: &mut WorkspaceRuntimeIdentity| identity.resource_prefix = "other".to_owned(),
            |identity: &mut WorkspaceRuntimeIdentity| identity.route_key = "other".to_owned(),
        ];
        for mutate in mutations {
            let mut tampered = identity.clone();
            mutate(&mut tampered);
            assert!(
                tampered
                    .validate_for_workspace(&installation, workspace_id, &short_id)
                    .is_err()
            );
        }

        let shared = WorkspaceRuntimeIdentity::prefixed_v2(
            &installation,
            workspace_id,
            &short_id,
            Some("persisted-shared-namespace"),
        )
        .unwrap();
        shared
            .validate_for_workspace(&installation, workspace_id, &short_id)
            .unwrap();
    }
}
