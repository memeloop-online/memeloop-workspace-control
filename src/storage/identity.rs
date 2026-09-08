use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::{ApiKeyScope, Permission, Role, RoleBinding};

use super::{ApiKeySummary, Database, StorageError, user_settings::validate_api_key_policy};

mod backend;
mod organizations;

use backend::{create_user_with_key_postgres, create_user_with_key_sqlite};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Principal {
    pub user_id: Uuid,
    pub display_name: String,
    pub system_admin: bool,
    pub memberships: Vec<Membership>,
    /// The authenticating key's explicit grants.
    pub api_key_scopes: Vec<ApiKeyScope>,
    /// The authenticating key's expiry timestamp.
    pub api_key_expires_at: Option<i64>,
    /// Optional allowlist of templates this key may use to create workspaces.
    /// `None` leaves template access governed by the user's normal permissions.
    pub allowed_template_ids: Option<Vec<Uuid>>,
}

impl Principal {
    pub fn allows(&self, permission: Permission, organization_id: Uuid) -> bool {
        if !self
            .api_key_scopes
            .iter()
            .any(|scope| scope.permits(permission))
        {
            return false;
        }
        if self.system_admin {
            return true;
        }
        self.memberships.iter().any(|membership| {
            RoleBinding {
                role: membership.role,
                organization_id: Some(membership.organization_id),
            }
            .allows(permission, organization_id)
        })
    }

    pub fn may_manage_api_keys(&self) -> bool {
        self.api_key_scopes
            .iter()
            .any(|scope| matches!(scope, ApiKeyScope::ManageApiKeys))
    }

    pub fn may_manage_system(&self) -> bool {
        self.system_admin && self.allows(Permission::ManageSystem, Uuid::nil())
    }

    pub fn may_use_template(&self, template_id: Uuid) -> bool {
        self.allowed_template_ids
            .as_ref()
            .is_none_or(|allowed| allowed.contains(&template_id))
    }

    pub fn may_access_workspace_template(&self, template_id: Option<Uuid>) -> bool {
        template_id.is_some_and(|id| self.may_use_template(id)) || !self.has_template_restriction()
    }

    pub fn has_template_restriction(&self) -> bool {
        self.allowed_template_ids.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Membership {
    pub organization_id: Uuid,
    pub role: Role,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Organization {
    pub id: Uuid,
    pub name: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateOrganization {
    pub name: String,
    pub owner_user_id: Uuid,
}

/// Inputs for creating a user with the first, bounded API key. Keeping these
/// together makes the onboarding operation difficult to call inconsistently.
pub struct InitialUserCommand<'a> {
    pub display_name: &'a str,
    pub token: &'a str,
    pub system_admin: bool,
    pub scopes: Vec<ApiKeyScope>,
    pub expires_at: i64,
    pub membership: Option<(Uuid, Role)>,
    pub now: i64,
}

struct UserWithKeyCommand<'a> {
    display_name: &'a str,
    token: &'a str,
    system_admin: bool,
    initial_key: ApiKeySummary,
    membership: Option<(Uuid, Role)>,
    now: i64,
}

impl Database {
    /// Creates a user with a safe, administrator-provided initial key policy.
    /// This deliberately accepts a caller-supplied token so onboarding can hand
    /// it to the user out of band without the API ever returning it.
    pub async fn create_user_with_initial_key(
        &self,
        display_name: &str,
        token: &str,
        system_admin: bool,
        scopes: Vec<ApiKeyScope>,
        expires_at: i64,
        now: i64,
    ) -> Result<Principal, StorageError> {
        self.create_user_with_initial_key_and_membership(InitialUserCommand {
            display_name,
            token,
            system_admin,
            scopes,
            expires_at,
            membership: None,
            now,
        })
        .await
    }

    /// Creates a user, its initial key, and (when supplied) an organization
    /// membership in one transaction. The organization is checked in that
    /// transaction so a missing target cannot leave a user or key behind.
    pub async fn create_user_with_initial_key_and_membership(
        &self,
        command: InitialUserCommand<'_>,
    ) -> Result<Principal, StorageError> {
        if command
            .membership
            .is_some_and(|(_, role)| role == Role::SystemAdmin)
        {
            return Err(StorageError::InvalidOrganizationMembership);
        }
        let scopes =
            validate_api_key_policy(command.scopes, Some(command.expires_at), command.now)?;
        self.create_user_with_key(UserWithKeyCommand {
            display_name: command.display_name,
            token: command.token,
            system_admin: command.system_admin,
            initial_key: ApiKeySummary {
                id: Uuid::now_v7(),
                name: "Initial key".to_owned(),
                prefix: token_prefix(command.token),
                last_used_at: None,
                created_at: command.now,
                scopes,
                expires_at: Some(command.expires_at),
                allowed_template_ids: None,
                revoked_at: None,
            },
            membership: command.membership,
            now: command.now,
        })
        .await
    }

    async fn create_user_with_key(
        &self,
        command: UserWithKeyCommand<'_>,
    ) -> Result<Principal, StorageError> {
        validate_token(command.token)?;
        let user_id = Uuid::now_v7();
        let token_hash = hash_token(command.token);
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                create_user_with_key_sqlite(
                    pool,
                    installation_id.as_str(),
                    user_id,
                    &token_hash,
                    &command,
                )
                .await?
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                create_user_with_key_postgres(
                    pool,
                    installation_id.as_str(),
                    user_id,
                    &token_hash,
                    &command,
                )
                .await?
            }
        }
        Ok(Principal {
            user_id,
            display_name: command.display_name.to_owned(),
            system_admin: command.system_admin,
            memberships: command
                .membership
                .map(|(organization_id, role)| Membership {
                    organization_id,
                    role,
                })
                .into_iter()
                .collect(),
            api_key_scopes: command.initial_key.scopes,
            api_key_expires_at: command.initial_key.expires_at,
            allowed_template_ids: command.initial_key.allowed_template_ids,
        })
    }

    pub async fn authenticate(&self, token: &str) -> Result<Option<Principal>, StorageError> {
        if token.len() < 32 {
            return Ok(None);
        }
        let token_hash = hash_token(token);
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let row = sqlx::query(
                    "SELECT u.id, u.display_name, u.system_admin, k.id AS key_id, \
                    k.last_used_at, k.scopes_json, k.expires_at, k.allowed_template_ids_json FROM users u \
                    JOIN user_api_keys k ON k.installation_id = u.installation_id AND k.user_id = u.id \
                    WHERE u.installation_id = ?1 AND k.token_hash = ?2 AND k.revoked_at IS NULL \
                    AND instr(k.scopes_json, '\"*\"') = 0 \
                    AND k.expires_at > unixepoch() AND u.disabled = 0",
                )
                .bind(installation_id.as_str())
                .bind(token_hash)
                .fetch_optional(pool)
                .await?;
                let Some(row) = row else { return Ok(None) };
                let user_id = Uuid::parse_str(row.try_get::<String, _>("id")?.as_str())?;
                let key_id: String = row.try_get("key_id")?;
                backend::mark_key_used_sqlite(
                    pool,
                    installation_id.as_str(),
                    &key_id,
                    row.try_get("last_used_at")?,
                )
                .await?;
                let memberships =
                    backend::sqlite_memberships(pool, installation_id.as_str(), user_id).await?;
                Ok(Some(Principal {
                    user_id,
                    display_name: row.try_get("display_name")?,
                    system_admin: row.try_get::<i64, _>("system_admin")? != 0,
                    memberships,
                    api_key_scopes: decode_scopes(row.try_get("scopes_json")?)?,
                    api_key_expires_at: row.try_get("expires_at")?,
                    allowed_template_ids: decode_allowed_template_ids(
                        row.try_get("allowed_template_ids_json")?,
                    )?,
                }))
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let row = sqlx::query(
                    "SELECT u.id, u.display_name, u.system_admin, k.id AS key_id, \
                    k.last_used_at, k.scopes_json, k.expires_at, k.allowed_template_ids_json FROM users u \
                    JOIN user_api_keys k ON k.installation_id = u.installation_id AND k.user_id = u.id \
                    WHERE u.installation_id = $1 AND k.token_hash = $2 AND k.revoked_at IS NULL \
                    AND position('\"*\"' IN k.scopes_json) = 0 \
                    AND k.expires_at > EXTRACT(EPOCH FROM CURRENT_TIMESTAMP)::BIGINT AND u.disabled = 0",
                )
                .bind(installation_id.as_str())
                .bind(token_hash)
                .fetch_optional(pool)
                .await?;
                let Some(row) = row else { return Ok(None) };
                let user_id = Uuid::parse_str(row.try_get::<String, _>("id")?.as_str())?;
                let key_id: String = row.try_get("key_id")?;
                backend::mark_key_used_postgres(
                    pool,
                    installation_id.as_str(),
                    &key_id,
                    row.try_get("last_used_at")?,
                )
                .await?;
                let memberships =
                    backend::postgres_memberships(pool, installation_id.as_str(), user_id).await?;
                Ok(Some(Principal {
                    user_id,
                    display_name: row.try_get("display_name")?,
                    system_admin: row.try_get::<i64, _>("system_admin")? != 0,
                    memberships,
                    api_key_scopes: decode_scopes(row.try_get("scopes_json")?)?,
                    api_key_expires_at: row.try_get("expires_at")?,
                    allowed_template_ids: decode_allowed_template_ids(
                        row.try_get("allowed_template_ids_json")?,
                    )?,
                }))
            }
        }
    }
}

fn decode_scopes(value: String) -> Result<Vec<ApiKeyScope>, StorageError> {
    let scopes: Vec<ApiKeyScope> = serde_json::from_str(&value)?;
    if scopes.is_empty() {
        return Err(StorageError::InvalidApiKey);
    }
    Ok(scopes)
}

fn decode_allowed_template_ids(value: Option<String>) -> Result<Option<Vec<Uuid>>, StorageError> {
    value
        .map(|json| serde_json::from_str(&json).map_err(StorageError::from))
        .transpose()
}

pub(super) fn hash_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn validate_token(token: &str) -> Result<(), StorageError> {
    if token.len() < 32 {
        return Err(StorageError::TokenTooShort);
    }
    Ok(())
}

pub(super) fn as_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::LeaseDurationOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_allowlist_distinguishes_unrestricted_empty_and_selected() {
        let template_id = Uuid::now_v7();
        let other_template_id = Uuid::now_v7();
        let mut principal = Principal {
            user_id: Uuid::now_v7(),
            display_name: "test".to_owned(),
            system_admin: false,
            memberships: Vec::new(),
            api_key_scopes: vec![ApiKeyScope::CreateWorkspace],
            api_key_expires_at: None,
            allowed_template_ids: None,
        };
        assert!(principal.may_use_template(template_id));
        assert!(principal.may_access_workspace_template(None));
        principal.allowed_template_ids = Some(Vec::new());
        assert!(!principal.may_use_template(template_id));
        assert!(!principal.may_access_workspace_template(None));
        principal.allowed_template_ids = Some(vec![template_id]);
        assert!(principal.may_use_template(template_id));
        assert!(!principal.may_use_template(other_template_id));
    }
}
