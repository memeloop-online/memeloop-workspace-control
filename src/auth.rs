use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    SystemAdmin,
    OrganizationAdmin,
    Member,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    ManageSystem,
    ManageOrganization,
    ManageMembers,
    ManageLockedInjections,
    CreateWorkspace,
    ReadWorkspace,
    ConnectWorkspace,
    ChangeWorkspaceState,
    DeleteWorkspace,
}

/// Stable, persisted API-key grants. `Wildcard` exists exclusively for keys created before
/// scopes were introduced and must never be accepted in a new-key request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyScope {
    #[serde(rename = "*")]
    Wildcard,
    ManageApiKeys,
    ManageSystem,
    ManageOrganization,
    ManageMembers,
    ManageLockedInjections,
    CreateWorkspace,
    ReadWorkspace,
    ConnectWorkspace,
    ChangeWorkspaceState,
    DeleteWorkspace,
}

impl ApiKeyScope {
    pub fn permits(self, permission: Permission) -> bool {
        matches!(self, Self::Wildcard)
            || matches!(
                (self, permission),
                (Self::ManageSystem, Permission::ManageSystem)
                    | (Self::ManageOrganization, Permission::ManageOrganization)
                    | (Self::ManageMembers, Permission::ManageMembers)
                    | (
                        Self::ManageLockedInjections,
                        Permission::ManageLockedInjections
                    )
                    | (Self::CreateWorkspace, Permission::CreateWorkspace)
                    | (Self::ReadWorkspace, Permission::ReadWorkspace)
                    | (Self::ConnectWorkspace, Permission::ConnectWorkspace)
                    | (Self::ChangeWorkspaceState, Permission::ChangeWorkspaceState)
                    | (Self::DeleteWorkspace, Permission::DeleteWorkspace)
            )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleBinding {
    pub role: Role,
    pub organization_id: Option<Uuid>,
}

impl RoleBinding {
    pub fn allows(&self, permission: Permission, resource_organization_id: Uuid) -> bool {
        match self.role {
            Role::SystemAdmin => true,
            Role::OrganizationAdmin if self.organization_id == Some(resource_organization_id) => {
                !matches!(permission, Permission::ManageSystem)
            }
            Role::Member if self.organization_id == Some(resource_organization_id) => matches!(
                permission,
                Permission::CreateWorkspace
                    | Permission::ReadWorkspace
                    | Permission::ConnectWorkspace
                    | Permission::ChangeWorkspaceState
            ),
            Role::OrganizationAdmin | Role::Member => false,
        }
    }
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SystemAdmin => "system_admin",
            Self::OrganizationAdmin => "organization_admin",
            Self::Member => "member",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "system_admin" => Some(Self::SystemAdmin),
            "organization_admin" => Some(Self::OrganizationAdmin),
            "member" => Some(Self::Member),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn organization_roles_never_cross_organization_boundary() {
        let own_organization = Uuid::now_v7();
        let other_organization = Uuid::now_v7();
        let binding = RoleBinding {
            role: Role::OrganizationAdmin,
            organization_id: Some(own_organization),
        };

        assert!(binding.allows(Permission::DeleteWorkspace, own_organization));
        assert!(!binding.allows(Permission::ReadWorkspace, other_organization));
        assert!(!binding.allows(Permission::ManageSystem, own_organization));
    }

    #[test]
    fn member_cannot_delete_workspace_or_lock_injection() {
        let organization = Uuid::now_v7();
        let binding = RoleBinding {
            role: Role::Member,
            organization_id: Some(organization),
        };

        assert!(binding.allows(Permission::CreateWorkspace, organization));
        assert!(!binding.allows(Permission::DeleteWorkspace, organization));
        assert!(!binding.allows(Permission::ManageLockedInjections, organization));
    }

    #[test]
    fn api_key_scopes_are_permission_specific_except_legacy_wildcard() {
        assert!(ApiKeyScope::ReadWorkspace.permits(Permission::ReadWorkspace));
        assert!(!ApiKeyScope::ReadWorkspace.permits(Permission::DeleteWorkspace));
        assert!(!ApiKeyScope::ManageApiKeys.permits(Permission::ManageSystem));
        assert!(ApiKeyScope::Wildcard.permits(Permission::ManageSystem));
        assert_eq!(
            serde_json::from_str::<Vec<ApiKeyScope>>(r#"["*"]"#).unwrap(),
            vec![ApiKeyScope::Wildcard]
        );
    }
}
