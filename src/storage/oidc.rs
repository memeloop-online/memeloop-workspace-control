use uuid::Uuid;

use crate::auth::{ApiKeyScope, Role};

use super::{Database, Membership, Principal, StorageError, identity::hash_token};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OidcBindingResult {
    Changed,
    Unauthorized,
    Conflict,
}

const OWNER: &str = "SELECT u.id, u.system_admin, k.scopes_json,
    CASE WHEN EXISTS (
        SELECT 1 FROM organization_memberships m
        WHERE m.installation_id = u.installation_id AND m.user_id = u.id AND m.role = 'organization_admin'
    ) THEN CAST(1 AS BIGINT) ELSE CAST(0 AS BIGINT) END
    FROM users u JOIN user_api_keys k
    ON k.installation_id = u.installation_id AND k.user_id = u.id AND k.token_hash = u.token_hash
    WHERE u.installation_id = $1 AND u.token_hash = $2 AND u.disabled = 0
    AND k.revoked_at IS NULL AND k.expires_at > $3 AND k.allowed_template_ids_json IS NULL";
const BIND: &str = "INSERT INTO oidc_identities
    (installation_id, issuer, subject, user_id, id, scopes_json, created_at)
    VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT DO NOTHING";
const UNBIND: &str = "DELETE FROM oidc_identities
    WHERE installation_id = $1 AND issuer = $2 AND subject = $3 AND user_id = $4";
const AUDIT: &str = "INSERT INTO audit_log
    (id, installation_id, actor_user_id, action, metadata_json, created_at) VALUES ($1, $2, $3, $4, $5, $6)";
const NEW_SESSION: &str =
    "INSERT INTO oidc_sessions (installation_id, session_hash, identity_id, expires_at)
    SELECT i.installation_id, $1, i.id, $2 FROM oidc_identities i
    JOIN users u ON u.installation_id = i.installation_id AND u.id = i.user_id
    WHERE i.installation_id = $3 AND i.issuer = $4 AND i.subject = $5 AND u.disabled = 0";
const LOCK_SESSION_USER: &str = "SELECT u.id FROM oidc_identities i
    JOIN users u ON u.installation_id = i.installation_id AND u.id = i.user_id
    WHERE i.installation_id = $1 AND i.issuer = $2 AND i.subject = $3 AND u.disabled = 0
    FOR SHARE OF u";
const SESSION: &str = "SELECT u.id, u.display_name, u.system_admin, i.scopes_json, m.organization_id, m.role
    FROM oidc_sessions s
    JOIN oidc_identities i ON i.installation_id = s.installation_id AND i.id = s.identity_id
    JOIN users u ON u.installation_id = i.installation_id AND u.id = i.user_id
    LEFT JOIN organization_memberships m ON m.installation_id = u.installation_id AND m.user_id = u.id
    WHERE s.installation_id = $1 AND s.session_hash = $2 AND i.issuer = $3
    AND s.expires_at > $4 AND u.disabled = 0
    ORDER BY m.organization_id";
const EXPIRE_SESSIONS: &str =
    "DELETE FROM oidc_sessions WHERE installation_id = $1 AND expires_at <= $2";
const REVOKE_SESSION: &str =
    "DELETE FROM oidc_sessions WHERE installation_id = $1 AND session_hash = $2";
const EXPIRE_FLOWS: &str = "DELETE FROM oidc_flows WHERE installation_id = $1 AND expires_at <= $2";
const PUT_FLOW: &str = "INSERT INTO oidc_flows (installation_id, flow_hash, payload, expires_at)
    VALUES ($1, $2, $3, $4)";
const TAKE_FLOW: &str = "DELETE FROM oidc_flows
    WHERE installation_id = $1 AND flow_hash = $2 AND expires_at > $3 RETURNING payload";

type OwnerRow = (String, i64, String, i64);
type SessionRow = (String, String, i64, String, Option<String>, Option<String>);

struct Owner {
    id: Uuid,
    grants: Vec<ApiKeyScope>,
}

fn checked_owner(row: Option<OwnerRow>) -> Result<Option<Owner>, StorageError> {
    let Some((id, system_admin, grants, org_admin)) = row else {
        return Ok(None);
    };
    let Ok(mut grants) = serde_json::from_str::<Vec<ApiKeyScope>>(&grants) else {
        return Ok(None);
    };
    if !ApiKeyScope::initial_key_defaults(system_admin != 0)
        .iter()
        .all(|scope| grants.contains(scope))
    {
        return Ok(None);
    }
    let ceiling = role_scopes(system_admin != 0, org_admin != 0);
    grants.retain(|scope| *scope != ApiKeyScope::ManageApiKeys && ceiling.contains(scope));
    Ok(Some(Owner {
        id: Uuid::parse_str(&id)?,
        grants,
    }))
}

fn session_principal(rows: Vec<SessionRow>) -> Result<Option<Principal>, StorageError> {
    let Some((id, name, admin, grants_json, _, _)) = rows.first() else {
        return Ok(None);
    };
    let mut grants = serde_json::from_str::<Vec<ApiKeyScope>>(grants_json)?;
    let mut memberships = Vec::new();
    for (_, _, _, _, organization_id, role) in &rows {
        if let (Some(id), Some(role)) = (organization_id, role) {
            memberships.push(Membership {
                organization_id: Uuid::parse_str(id)?,
                role: Role::from_database(role)
                    .ok_or_else(|| StorageError::UnknownRole(role.clone()))?,
            });
        }
    }
    // An account promoted later still needs a new, appropriately scoped proof to
    // use system-administrator access from its OIDC binding.
    let system_admin = *admin != 0 && grants.contains(&ApiKeyScope::ManageSystem);
    let ceiling = role_scopes(
        system_admin,
        memberships
            .iter()
            .any(|m| m.role == Role::OrganizationAdmin),
    );
    grants.retain(|scope| *scope != ApiKeyScope::ManageApiKeys && ceiling.contains(scope));
    Ok(Some(Principal {
        user_id: Uuid::parse_str(id)?,
        display_name: name.clone(),
        system_admin,
        memberships,
        api_key_scopes: grants,
        api_key_expires_at: None,
        allowed_template_ids: None,
    }))
}

fn role_scopes(system_admin: bool, organization_admin: bool) -> Vec<ApiKeyScope> {
    let mut scopes = ApiKeyScope::initial_key_defaults(system_admin);
    if organization_admin && !system_admin {
        scopes.extend([
            ApiKeyScope::ManageOrganization,
            ApiKeyScope::ManageMembers,
            ApiKeyScope::ManageLockedInjections,
            ApiKeyScope::DeleteWorkspace,
        ]);
    }
    scopes
}

impl Database {
    pub async fn oidc_owner(&self, token: &str) -> Result<Option<Uuid>, StorageError> {
        let now = super::migration::unix_timestamp()?;
        let token_hash = hash_token(token);
        let installation = self.installation_id().as_str();
        let row: Option<OwnerRow> = match self {
            Self::Sqlite { pool, .. } => {
                sqlx::query_as(OWNER)
                    .bind(installation)
                    .bind(&token_hash)
                    .bind(now)
                    .fetch_optional(pool)
                    .await?
            }
            Self::Postgres { pool, .. } => {
                sqlx::query_as(OWNER)
                    .bind(installation)
                    .bind(&token_hash)
                    .bind(now)
                    .fetch_optional(pool)
                    .await?
            }
        };
        Ok(checked_owner(row)?.map(|owner| owner.id))
    }

    pub async fn oidc_bind(
        &self,
        token: &str,
        issuer: &str,
        subject: &str,
        now: i64,
    ) -> Result<OidcBindingResult, StorageError> {
        self.change_oidc_binding(token, issuer, subject, now, true)
            .await
    }

    pub async fn oidc_unbind(
        &self,
        token: &str,
        issuer: &str,
        subject: &str,
        now: i64,
    ) -> Result<OidcBindingResult, StorageError> {
        self.change_oidc_binding(token, issuer, subject, now, false)
            .await
    }

    async fn change_oidc_binding(
        &self,
        token: &str,
        issuer: &str,
        subject: &str,
        now: i64,
        bind: bool,
    ) -> Result<OidcBindingResult, StorageError> {
        let installation = self.installation_id().as_str();
        let token_hash = hash_token(token);
        let action = if bind {
            "user.oidc.bind"
        } else {
            "user.oidc.unbind"
        };
        let result = match self {
            Self::Sqlite { pool, .. } => {
                let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
                let row = sqlx::query_as(OWNER)
                    .bind(installation)
                    .bind(&token_hash)
                    .bind(now)
                    .fetch_optional(&mut *tx)
                    .await?;
                let Some(owner) = checked_owner(row)? else {
                    return Ok(OidcBindingResult::Unauthorized);
                };
                let user_id = owner.id.to_string();
                let statement = if bind {
                    sqlx::query(BIND)
                        .bind(installation)
                        .bind(issuer)
                        .bind(subject)
                        .bind(&user_id)
                        .bind(Uuid::now_v7().to_string())
                        .bind(serde_json::to_string(&owner.grants)?)
                        .bind(now)
                } else {
                    sqlx::query(UNBIND)
                        .bind(installation)
                        .bind(issuer)
                        .bind(subject)
                        .bind(&user_id)
                };
                let changed = statement.execute(&mut *tx).await?.rows_affected() == 1;
                if changed {
                    sqlx::query(AUDIT)
                        .bind(Uuid::now_v7().to_string())
                        .bind(installation)
                        .bind(&user_id)
                        .bind(action)
                        .bind(audit_metadata(issuer, subject))
                        .bind(now)
                        .execute(&mut *tx)
                        .await?;
                }
                tx.commit().await?;
                changed
            }
            Self::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                // API-key revocation and account changes also lock the user before writing.
                let row = sqlx::query_as(&format!("{OWNER} FOR UPDATE OF u, k"))
                    .bind(installation)
                    .bind(&token_hash)
                    .bind(now)
                    .fetch_optional(&mut *tx)
                    .await?;
                let Some(owner) = checked_owner(row)? else {
                    return Ok(OidcBindingResult::Unauthorized);
                };
                let user_id = owner.id.to_string();
                let statement = if bind {
                    sqlx::query(BIND)
                        .bind(installation)
                        .bind(issuer)
                        .bind(subject)
                        .bind(&user_id)
                        .bind(Uuid::now_v7().to_string())
                        .bind(serde_json::to_string(&owner.grants)?)
                        .bind(now)
                } else {
                    sqlx::query(UNBIND)
                        .bind(installation)
                        .bind(issuer)
                        .bind(subject)
                        .bind(&user_id)
                };
                let changed = statement.execute(&mut *tx).await?.rows_affected() == 1;
                if changed {
                    sqlx::query(AUDIT)
                        .bind(Uuid::now_v7().to_string())
                        .bind(installation)
                        .bind(&user_id)
                        .bind(action)
                        .bind(audit_metadata(issuer, subject))
                        .bind(now)
                        .execute(&mut *tx)
                        .await?;
                }
                tx.commit().await?;
                changed
            }
        };
        Ok(if result {
            OidcBindingResult::Changed
        } else {
            OidcBindingResult::Conflict
        })
    }

    pub async fn oidc_create_session(
        &self,
        issuer: &str,
        subject: &str,
        session_hash: &str,
        expires_at: i64,
    ) -> Result<bool, StorageError> {
        let installation = self.installation_id().as_str();
        let now = super::migration::unix_timestamp()?;
        if expires_at <= now {
            return Ok(false);
        }
        let rows = match self {
            Self::Sqlite { pool, .. } => sqlx::query(NEW_SESSION)
                .bind(session_hash)
                .bind(expires_at)
                .bind(installation)
                .bind(issuer)
                .bind(subject)
                .execute(pool)
                .await?
                .rows_affected(),
            Self::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                // Serialize with account disable and unlink, both of which lock the same
                // user before deleting sessions. Recheck the binding in the next statement.
                let Some(user_id) = sqlx::query_scalar::<_, String>(LOCK_SESSION_USER)
                    .bind(installation)
                    .bind(issuer)
                    .bind(subject)
                    .fetch_optional(&mut *tx)
                    .await?
                else {
                    return Ok(false);
                };
                let rows = sqlx::query(&format!("{NEW_SESSION} AND u.id = $6"))
                    .bind(session_hash)
                    .bind(expires_at)
                    .bind(installation)
                    .bind(issuer)
                    .bind(subject)
                    .bind(user_id)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected();
                tx.commit().await?;
                rows
            }
        };
        Ok(rows == 1)
    }

    pub async fn oidc_session_principal(
        &self,
        session_hash: &str,
        issuer: &str,
        now: i64,
    ) -> Result<Option<Principal>, StorageError> {
        let installation = self.installation_id().as_str();
        let rows = match self {
            Self::Sqlite { pool, .. } => {
                sqlx::query_as(SESSION)
                    .bind(installation)
                    .bind(session_hash)
                    .bind(issuer)
                    .bind(now)
                    .fetch_all(pool)
                    .await?
            }
            Self::Postgres { pool, .. } => {
                sqlx::query_as(SESSION)
                    .bind(installation)
                    .bind(session_hash)
                    .bind(issuer)
                    .bind(now)
                    .fetch_all(pool)
                    .await?
            }
        };
        session_principal(rows)
    }

    pub async fn oidc_revoke_session(&self, session_hash: &str) -> Result<(), StorageError> {
        let installation = self.installation_id().as_str();
        match self {
            Self::Sqlite { pool, .. } => {
                sqlx::query(REVOKE_SESSION)
                    .bind(installation)
                    .bind(session_hash)
                    .execute(pool)
                    .await?;
            }
            Self::Postgres { pool, .. } => {
                sqlx::query(REVOKE_SESSION)
                    .bind(installation)
                    .bind(session_hash)
                    .execute(pool)
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn oidc_put_flow(
        &self,
        flow_hash: &str,
        payload: &str,
        expires_at: i64,
        now: i64,
    ) -> Result<(), StorageError> {
        let installation = self.installation_id().as_str();
        match self {
            Self::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                sqlx::query(EXPIRE_FLOWS)
                    .bind(installation)
                    .bind(now)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query(EXPIRE_SESSIONS)
                    .bind(installation)
                    .bind(now)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query(PUT_FLOW)
                    .bind(installation)
                    .bind(flow_hash)
                    .bind(payload)
                    .bind(expires_at)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
            }
            Self::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                sqlx::query(EXPIRE_FLOWS)
                    .bind(installation)
                    .bind(now)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query(EXPIRE_SESSIONS)
                    .bind(installation)
                    .bind(now)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query(PUT_FLOW)
                    .bind(installation)
                    .bind(flow_hash)
                    .bind(payload)
                    .bind(expires_at)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
            }
        }
        Ok(())
    }

    pub async fn oidc_take_flow(
        &self,
        flow_hash: &str,
        now: i64,
    ) -> Result<Option<String>, StorageError> {
        let installation = self.installation_id().as_str();
        Ok(match self {
            Self::Sqlite { pool, .. } => {
                sqlx::query_scalar(TAKE_FLOW)
                    .bind(installation)
                    .bind(flow_hash)
                    .bind(now)
                    .fetch_optional(pool)
                    .await?
            }
            Self::Postgres { pool, .. } => {
                sqlx::query_scalar(TAKE_FLOW)
                    .bind(installation)
                    .bind(flow_hash)
                    .bind(now)
                    .fetch_optional(pool)
                    .await?
            }
        })
    }
}

fn audit_metadata(issuer: &str, subject: &str) -> String {
    serde_json::json!({ "issuer": issuer, "subject_sha256": hash_token(subject) }).to_string()
}
