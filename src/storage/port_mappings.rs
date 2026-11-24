//! Durable, installation-scoped HTTP port mappings.
//!
//! A mapping is intentionally only a declaration.  The API authorizes its
//! creation and the reconciler materializes the corresponding Kubernetes
//! objects.  Keeping that split means a failed Kubernetes write never causes
//! an unowned public Service to be created.

use std::fmt;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, SqliteConnection};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{Database, StorageError};
use crate::{templates::is_platform_reserved_port, workspaces::Workspace};

pub(crate) const PORT_MAPPING_RECONCILE_JOB: &str = "reconcile_port_mappings";

/// HTTP ports which a workspace is allowed to publish through the authenticated
/// gateway.  Port 80 is deliberately allowed: it is a *container* port behind
/// a ClusterIP service, never a host port or NodePort.
pub fn validate_http_port(port: u16) -> Result<(), StorageError> {
    let allowed = port == 80 || port == 443 || (1024..=65535).contains(&port);
    // These are control-plane / workspace platform listeners, not application
    // ports.  Keeping the deny-list here makes every caller enforce it.
    let reserved = is_platform_reserved_port(port);
    if !allowed || reserved {
        return Err(StorageError::InvalidPortMappingPort);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct PortMapping {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub workspace_id: Uuid,
    pub internal_port: u16,
    pub display_name: Option<String>,
    pub subdomain: String,
    pub created_by: Uuid,
    pub created_at: i64,
}

impl PortMapping {
    pub fn hostname(&self, domain: &str) -> String {
        format!("p-{}.{}", self.subdomain, domain)
    }
}

#[derive(Clone, Serialize, ToSchema)]
pub struct IssuedPortMappingTicket {
    pub ticket: String,
    pub expires_at: i64,
}

impl fmt::Debug for IssuedPortMappingTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IssuedPortMappingTicket")
            .field("ticket", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl Database {
    /// Ensures the template-snapshot desktop endpoint has exactly one mapping.
    ///
    /// The row intentionally shares the ordinary mapping table so it follows
    /// the same authenticated HTTPS, service, ingress, policy, and ticket
    /// lifecycle. Its controller ownership is derived from the immutable
    /// workspace template snapshot, rather than mutable user-supplied state.
    pub async fn ensure_desktop_port_mapping(
        &self,
        workspace: &Workspace,
        now: i64,
    ) -> Result<Option<PortMapping>, StorageError> {
        let Some(desktop) = workspace.template.desktop.as_ref() else {
            return Ok(None);
        };
        self.insert_port_mapping(
            workspace.organization_id,
            workspace.id,
            desktop.internal_port,
            desktop.display_name.as_deref(),
            None,
            workspace.owner_id,
            now,
            false,
        )
        .await
        .map(Some)
    }

    /// Inserts a mapping exactly once per workspace/port.  Concurrent requests
    /// (including requests reaching different control-plane replicas) converge
    /// on the same row rather than allocating duplicate public resources.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_port_mapping(
        &self,
        organization_id: Uuid,
        workspace_id: Uuid,
        internal_port: u16,
        display_name: Option<&str>,
        subdomain: Option<&str>,
        created_by: Uuid,
        created_at: i64,
    ) -> Result<PortMapping, StorageError> {
        self.insert_port_mapping(
            organization_id,
            workspace_id,
            internal_port,
            display_name,
            subdomain,
            created_by,
            created_at,
            true,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_port_mapping(
        &self,
        organization_id: Uuid,
        workspace_id: Uuid,
        internal_port: u16,
        display_name: Option<&str>,
        subdomain: Option<&str>,
        created_by: Uuid,
        created_at: i64,
        reconcile: bool,
    ) -> Result<PortMapping, StorageError> {
        validate_http_port(internal_port)?;
        let display_name = validate_display_name(display_name)?;
        let custom_subdomain = subdomain.map(normalize_subdomain).transpose()?;
        let id = Uuid::now_v7();
        // Only an actual random-label collision retries; user requests and
        // unrelated database errors are never retried here.
        for _ in 0..4 {
            let subdomain = match &custom_subdomain {
                Some(value) => value.clone(),
                None => random_subdomain()?,
            };
            let inserted: Result<(), StorageError> = async {
                match self {
                    Self::Sqlite { pool, installation_id } => {
                        let mut tx = pool.begin().await?;
                        let claim: Option<String> = sqlx::query_scalar("INSERT INTO workspace_port_subdomain_claims (subdomain, installation_id, workspace_id) VALUES (?1, ?2, ?3) ON CONFLICT (subdomain) DO UPDATE SET subdomain = excluded.subdomain WHERE workspace_port_subdomain_claims.installation_id = excluded.installation_id AND workspace_port_subdomain_claims.workspace_id = excluded.workspace_id RETURNING subdomain")
                            .bind(&subdomain).bind(installation_id.as_str()).bind(workspace_id.to_string())
                            .fetch_optional(&mut *tx).await?;
                        if claim.is_none() { return Err(StorageError::PortMappingSubdomainConflict); }
                        let rows = sqlx::query("INSERT INTO workspace_port_mappings (id, installation_id, organization_id, workspace_id, internal_port, display_name, subdomain, created_by, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT (installation_id, workspace_id, internal_port) DO NOTHING")
                            .bind(id.to_string()).bind(installation_id.as_str()).bind(organization_id.to_string())
                            .bind(workspace_id.to_string()).bind(i64::from(internal_port)).bind(&display_name)
                            .bind(&subdomain).bind(created_by.to_string()).bind(created_at).execute(&mut *tx).await
                            .map_err(subdomain_write_error)?.rows_affected();
                        if rows == 0 {
                            tx.rollback().await?;
                        } else {
                            if reconcile {
                                enqueue_routes_sqlite(&mut tx, installation_id.as_str(), workspace_id, created_at).await?;
                            }
                            tx.commit().await?;
                        }
                    }
                    Self::Postgres { pool, installation_id } => {
                        let mut tx = pool.begin().await?;
                        let claim: Option<String> = sqlx::query_scalar("INSERT INTO workspace_port_subdomain_claims (subdomain, installation_id, workspace_id) VALUES ($1, $2, $3) ON CONFLICT (subdomain) DO UPDATE SET subdomain = excluded.subdomain WHERE workspace_port_subdomain_claims.installation_id = excluded.installation_id AND workspace_port_subdomain_claims.workspace_id = excluded.workspace_id RETURNING subdomain")
                            .bind(&subdomain).bind(installation_id.as_str()).bind(workspace_id.to_string())
                            .fetch_optional(&mut *tx).await?;
                        if claim.is_none() { return Err(StorageError::PortMappingSubdomainConflict); }
                        let rows = sqlx::query("INSERT INTO workspace_port_mappings (id, installation_id, organization_id, workspace_id, internal_port, display_name, subdomain, created_by, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) ON CONFLICT (installation_id, workspace_id, internal_port) DO NOTHING")
                            .bind(id.to_string()).bind(installation_id.as_str()).bind(organization_id.to_string())
                            .bind(workspace_id.to_string()).bind(i64::from(internal_port)).bind(&display_name)
                            .bind(&subdomain).bind(created_by.to_string()).bind(created_at).execute(&mut *tx).await
                            .map_err(subdomain_write_error)?.rows_affected();
                        if rows == 0 {
                            tx.rollback().await?;
                        } else {
                            if reconcile {
                                enqueue_routes_postgres(&mut tx, installation_id.as_str(), workspace_id, created_at).await?;
                            }
                            tx.commit().await?;
                        }
                    }
                }
                Ok(())
            }.await;
            match inserted {
                Ok(_) => {
                    return self
                        .port_mapping_by_workspace_port(workspace_id, internal_port)
                        .await;
                }
                Err(StorageError::PortMappingSubdomainConflict) => {
                    // A simultaneous create for the same workspace/port still
                    // converges on the existing declaration.
                    match self
                        .port_mapping_by_workspace_port(workspace_id, internal_port)
                        .await
                    {
                        Ok(existing) => return Ok(existing),
                        Err(StorageError::PortMappingNotFound) => {}
                        Err(error) => return Err(error),
                    }
                    if custom_subdomain.is_some() {
                        return Err(StorageError::PortMappingSubdomainConflict);
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Err(StorageError::PortMappingSubdomainConflict)
    }

    /// Edits retain the mapping UUID and Kubernetes resource identities. A
    /// hostname or application-port change atomically revokes old credentials.
    pub async fn update_port_mapping(
        &self,
        workspace_id: Uuid,
        mapping_id: Uuid,
        internal_port: u16,
        display_name: Option<&str>,
        subdomain: Option<&str>,
        now: i64,
    ) -> Result<PortMapping, StorageError> {
        validate_http_port(internal_port)?;
        let display_name = validate_display_name(display_name)?;
        let subdomain = subdomain.map(normalize_subdomain).transpose()?;
        let result: Result<PortMapping, StorageError> = async {
            match self {
                Self::Sqlite { pool, installation_id } => {
                    let mut tx = pool.begin().await?;
                    if let Some(subdomain) = &subdomain {
                        let claim: Option<String> = sqlx::query_scalar("INSERT INTO workspace_port_subdomain_claims (subdomain, installation_id, workspace_id) SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM workspace_port_mappings WHERE installation_id = ?2 AND workspace_id = ?3 AND id = ?4) ON CONFLICT (subdomain) DO UPDATE SET subdomain = excluded.subdomain WHERE workspace_port_subdomain_claims.installation_id = excluded.installation_id AND workspace_port_subdomain_claims.workspace_id = excluded.workspace_id RETURNING subdomain")
                            .bind(subdomain).bind(installation_id.as_str()).bind(workspace_id.to_string()).bind(mapping_id.to_string())
                            .fetch_optional(&mut *tx).await?;
                        if claim.is_none() {
                            let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workspace_port_mappings WHERE installation_id = ?1 AND workspace_id = ?2 AND id = ?3")
                                .bind(installation_id.as_str()).bind(workspace_id.to_string()).bind(mapping_id.to_string()).fetch_one(&mut *tx).await?;
                            return Err(if exists == 0 { StorageError::PortMappingNotFound } else { StorageError::PortMappingSubdomainConflict });
                        }
                    }
                    for table in ["workspace_port_mapping_tickets", "workspace_port_mapping_sessions"] {
                        let sql = format!("DELETE FROM {table} WHERE installation_id = ?1 AND mapping_id = ?2 AND EXISTS (SELECT 1 FROM workspace_port_mappings WHERE installation_id = ?1 AND workspace_id = ?3 AND id = ?2 AND (internal_port <> ?4 OR (?5 IS NOT NULL AND subdomain <> ?5)))");
                        sqlx::query(&sql).bind(installation_id.as_str()).bind(mapping_id.to_string())
                            .bind(workspace_id.to_string()).bind(i64::from(internal_port)).bind(&subdomain).execute(&mut *tx).await?;
                    }
                    let changed = sqlx::query("UPDATE workspace_port_mappings SET internal_port = ?1, display_name = ?2, subdomain = COALESCE(?3, subdomain) WHERE installation_id = ?4 AND workspace_id = ?5 AND id = ?6 RETURNING id, organization_id, workspace_id, internal_port, display_name, subdomain, created_by, created_at")
                        .bind(i64::from(internal_port)).bind(&display_name).bind(&subdomain).bind(installation_id.as_str())
                        .bind(workspace_id.to_string()).bind(mapping_id.to_string()).fetch_optional(&mut *tx).await?;
                    let saved = decode(changed.ok_or(StorageError::PortMappingNotFound)?)?;
                    enqueue_routes_sqlite(&mut tx, installation_id.as_str(), workspace_id, now).await?;
                    tx.commit().await?;
                    Ok(saved)
                }
                Self::Postgres { pool, installation_id } => {
                    let mut tx = pool.begin().await?;
                    let existing = sqlx::query("SELECT id FROM workspace_port_mappings WHERE installation_id = $1 AND workspace_id = $2 AND id = $3 FOR UPDATE")
                        .bind(installation_id.as_str()).bind(workspace_id.to_string()).bind(mapping_id.to_string())
                        .fetch_optional(&mut *tx).await?;
                    if existing.is_none() { return Err(StorageError::PortMappingNotFound); }
                    if let Some(subdomain) = &subdomain {
                        let claim: Option<String> = sqlx::query_scalar("INSERT INTO workspace_port_subdomain_claims (subdomain, installation_id, workspace_id) VALUES ($1, $2, $3) ON CONFLICT (subdomain) DO UPDATE SET subdomain = excluded.subdomain WHERE workspace_port_subdomain_claims.installation_id = excluded.installation_id AND workspace_port_subdomain_claims.workspace_id = excluded.workspace_id RETURNING subdomain")
                            .bind(subdomain).bind(installation_id.as_str()).bind(workspace_id.to_string())
                            .fetch_optional(&mut *tx).await?;
                        if claim.is_none() { return Err(StorageError::PortMappingSubdomainConflict); }
                    }
                    for table in ["workspace_port_mapping_tickets", "workspace_port_mapping_sessions"] {
                        let sql = format!("DELETE FROM {table} WHERE installation_id = $1 AND mapping_id = $2 AND EXISTS (SELECT 1 FROM workspace_port_mappings WHERE installation_id = $1 AND workspace_id = $3 AND id = $2 AND (internal_port <> $4 OR ($5::text IS NOT NULL AND subdomain <> $5)))");
                        sqlx::query(&sql).bind(installation_id.as_str()).bind(mapping_id.to_string())
                            .bind(workspace_id.to_string()).bind(i64::from(internal_port)).bind(&subdomain).execute(&mut *tx).await?;
                    }
                    let changed = sqlx::query("UPDATE workspace_port_mappings SET internal_port = $1, display_name = $2, subdomain = COALESCE($3, subdomain) WHERE installation_id = $4 AND workspace_id = $5 AND id = $6 RETURNING id, organization_id, workspace_id, internal_port, display_name, subdomain, created_by, created_at")
                        .bind(i64::from(internal_port)).bind(&display_name).bind(&subdomain).bind(installation_id.as_str())
                        .bind(workspace_id.to_string()).bind(mapping_id.to_string()).fetch_optional(&mut *tx).await?;
                    let saved = decode(changed.ok_or(StorageError::PortMappingNotFound)?)?;
                    enqueue_routes_postgres(&mut tx, installation_id.as_str(), workspace_id, now).await?;
                    tx.commit().await?;
                    Ok(saved)
                }
            }
        }.await;
        match result {
            Err(StorageError::Database(error)) if is_subdomain_conflict(&error) => {
                Err(StorageError::PortMappingSubdomainConflict)
            }
            Err(StorageError::Database(error))
                if error
                    .as_database_error()
                    .is_some_and(|e| e.is_unique_violation()) =>
            {
                Err(StorageError::PortMappingPortConflict)
            }
            result => result,
        }
    }

    pub async fn get_port_mapping_by_subdomain(
        &self,
        subdomain: &str,
    ) -> Result<PortMapping, StorageError> {
        match self {
            Self::Sqlite { pool, installation_id } => decode(sqlx::query("SELECT id, organization_id, workspace_id, internal_port, display_name, subdomain, created_by, created_at FROM workspace_port_mappings WHERE installation_id = ?1 AND subdomain = ?2").bind(installation_id.as_str()).bind(subdomain).fetch_optional(pool).await?.ok_or(StorageError::PortMappingNotFound)?),
            Self::Postgres { pool, installation_id } => decode(sqlx::query("SELECT id, organization_id, workspace_id, internal_port, display_name, subdomain, created_by, created_at FROM workspace_port_mappings WHERE installation_id = $1 AND subdomain = $2").bind(installation_id.as_str()).bind(subdomain).fetch_optional(pool).await?.ok_or(StorageError::PortMappingNotFound)?),
        }
    }

    pub async fn list_port_mappings(
        &self,
        workspace_id: Uuid,
    ) -> Result<Vec<PortMapping>, StorageError> {
        let sql_sqlite = "SELECT id, organization_id, workspace_id, internal_port, display_name, subdomain, created_by, created_at FROM workspace_port_mappings WHERE installation_id = ?1 AND workspace_id = ?2 ORDER BY internal_port";
        let sql_postgres = "SELECT id, organization_id, workspace_id, internal_port, display_name, subdomain, created_by, created_at FROM workspace_port_mappings WHERE installation_id = $1 AND workspace_id = $2 ORDER BY internal_port";
        let mappings = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(sql_sqlite)
                .bind(installation_id.as_str())
                .bind(workspace_id.to_string())
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(decode)
                .collect::<Result<Vec<_>, _>>()?,
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(sql_postgres)
                .bind(installation_id.as_str())
                .bind(workspace_id.to_string())
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(decode)
                .collect::<Result<Vec<_>, _>>()?,
        };
        Ok(mappings)
    }

    pub async fn delete_port_mapping(
        &self,
        workspace_id: Uuid,
        mapping_id: Uuid,
        now: i64,
    ) -> Result<bool, StorageError> {
        let deleted = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut tx = pool.begin().await?;
                let deleted = sqlx::query("DELETE FROM workspace_port_mappings WHERE installation_id = ?1 AND workspace_id = ?2 AND id = ?3")
                    .bind(installation_id.as_str()).bind(workspace_id.to_string()).bind(mapping_id.to_string())
                    .execute(&mut *tx).await?.rows_affected() == 1;
                if deleted {
                    enqueue_routes_sqlite(&mut tx, installation_id.as_str(), workspace_id, now)
                        .await?;
                }
                tx.commit().await?;
                deleted
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut tx = pool.begin().await?;
                let deleted = sqlx::query("DELETE FROM workspace_port_mappings WHERE installation_id = $1 AND workspace_id = $2 AND id = $3")
                    .bind(installation_id.as_str()).bind(workspace_id.to_string()).bind(mapping_id.to_string())
                    .execute(&mut *tx).await?.rows_affected() == 1;
                if deleted {
                    enqueue_routes_postgres(&mut tx, installation_id.as_str(), workspace_id, now)
                        .await?;
                }
                tx.commit().await?;
                deleted
            }
        };
        Ok(deleted)
    }

    pub async fn get_port_mapping(&self, mapping_id: Uuid) -> Result<PortMapping, StorageError> {
        match self {
            Self::Sqlite { pool, installation_id } => decode(sqlx::query("SELECT id, organization_id, workspace_id, internal_port, display_name, subdomain, created_by, created_at FROM workspace_port_mappings WHERE installation_id = ?1 AND id = ?2").bind(installation_id.as_str()).bind(mapping_id.to_string()).fetch_optional(pool).await?.ok_or(StorageError::PortMappingNotFound)?),
            Self::Postgres { pool, installation_id } => decode(sqlx::query("SELECT id, organization_id, workspace_id, internal_port, display_name, subdomain, created_by, created_at FROM workspace_port_mappings WHERE installation_id = $1 AND id = $2").bind(installation_id.as_str()).bind(mapping_id.to_string()).fetch_optional(pool).await?.ok_or(StorageError::PortMappingNotFound)?),
        }
    }

    /// A ticket is a short-lived, one-use bootstrap credential.  It is only
    /// accepted by Higress external-auth, which exchanges it for an opaque
    /// HttpOnly cookie.  Neither a mapping URL nor a cookie contains a raw
    /// database credential.
    pub async fn issue_port_mapping_ticket(
        &self,
        mapping: &PortMapping,
        user_id: Uuid,
        now: i64,
    ) -> Result<IssuedPortMappingTicket, StorageError> {
        self.prune_expired_port_mapping_auth(now).await?;
        let expires_at = now.checked_add(60).ok_or(StorageError::InvalidTicketTtl)?;
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| StorageError::RandomSource)?;
        let ticket = URL_SAFE_NO_PAD.encode(bytes);
        let hash = hash_secret(&ticket);
        let id = Uuid::now_v7();
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO workspace_port_mapping_tickets (id, installation_id, mapping_id, user_id, ticket_hash, expires_at, consumed_at, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7)").bind(id.to_string()).bind(installation_id.as_str()).bind(mapping.id.to_string()).bind(user_id.to_string()).bind(&hash).bind(expires_at).bind(now).execute(pool).await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO workspace_port_mapping_tickets (id, installation_id, mapping_id, user_id, ticket_hash, expires_at, consumed_at, created_at) VALUES ($1, $2, $3, $4, $5, $6, NULL, $7)").bind(id.to_string()).bind(installation_id.as_str()).bind(mapping.id.to_string()).bind(user_id.to_string()).bind(&hash).bind(expires_at).bind(now).execute(pool).await?;
            }
        }
        Ok(IssuedPortMappingTicket { ticket, expires_at })
    }

    async fn prune_expired_port_mapping_auth(&self, now: i64) -> Result<(), StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query("DELETE FROM workspace_port_mapping_sessions WHERE installation_id = ?1 AND expires_at < ?2")
                    .bind(installation_id.as_str()).bind(now).execute(pool).await?;
                sqlx::query("DELETE FROM workspace_port_mapping_tickets WHERE installation_id = ?1 AND expires_at < ?2")
                    .bind(installation_id.as_str()).bind(now).execute(pool).await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query("DELETE FROM workspace_port_mapping_sessions WHERE installation_id = $1 AND expires_at < $2")
                    .bind(installation_id.as_str()).bind(now).execute(pool).await?;
                sqlx::query("DELETE FROM workspace_port_mapping_tickets WHERE installation_id = $1 AND expires_at < $2")
                    .bind(installation_id.as_str()).bind(now).execute(pool).await?;
            }
        }
        Ok(())
    }

    /// Atomically consumes a ticket and creates a session.  `session_hash` is
    /// generated by the caller and only its SHA-256 digest is persisted.
    pub async fn exchange_port_mapping_ticket(
        &self,
        mapping_id: Uuid,
        ticket: &str,
        session_hash: &str,
        now: i64,
        expires_at: i64,
    ) -> Result<Option<Uuid>, StorageError> {
        if ticket.len() < 32 {
            return Ok(None);
        }
        let ticket_hash = hash_secret(ticket);
        let session_id = Uuid::now_v7();
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let mut tx = pool.begin().await?;
                let user: Option<String> = sqlx::query_scalar("UPDATE workspace_port_mapping_tickets SET consumed_at = ?1 WHERE installation_id = ?2 AND mapping_id = ?3 AND ticket_hash = ?4 AND consumed_at IS NULL AND expires_at >= ?1 RETURNING user_id").bind(now).bind(installation_id.as_str()).bind(mapping_id.to_string()).bind(ticket_hash).fetch_optional(&mut *tx).await?;
                let Some(user) = user else {
                    tx.rollback().await?;
                    return Ok(None);
                };
                sqlx::query("INSERT INTO workspace_port_mapping_sessions (id, installation_id, mapping_id, user_id, session_hash, expires_at, revoked_at, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7)").bind(session_id.to_string()).bind(installation_id.as_str()).bind(mapping_id.to_string()).bind(user).bind(session_hash).bind(expires_at).bind(now).execute(&mut *tx).await?;
                tx.commit().await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut tx = pool.begin().await?;
                let user: Option<String> = sqlx::query_scalar("UPDATE workspace_port_mapping_tickets SET consumed_at = $1 WHERE installation_id = $2 AND mapping_id = $3 AND ticket_hash = $4 AND consumed_at IS NULL AND expires_at >= $1 RETURNING user_id").bind(now).bind(installation_id.as_str()).bind(mapping_id.to_string()).bind(ticket_hash).fetch_optional(&mut *tx).await?;
                let Some(user) = user else {
                    tx.rollback().await?;
                    return Ok(None);
                };
                sqlx::query("INSERT INTO workspace_port_mapping_sessions (id, installation_id, mapping_id, user_id, session_hash, expires_at, revoked_at, created_at) VALUES ($1, $2, $3, $4, $5, $6, NULL, $7)").bind(session_id.to_string()).bind(installation_id.as_str()).bind(mapping_id.to_string()).bind(user).bind(session_hash).bind(expires_at).bind(now).execute(&mut *tx).await?;
                tx.commit().await?;
            }
        };
        Ok(Some(session_id))
    }

    pub async fn port_mapping_session_valid(
        &self,
        mapping_id: Uuid,
        session_hash: &str,
        now: i64,
    ) -> Result<bool, StorageError> {
        let found: Option<i64> = match self {
            Self::Sqlite { pool, installation_id } => sqlx::query_scalar(
                "SELECT 1 FROM workspace_port_mapping_sessions session \
                 JOIN workspace_port_mappings mapping ON mapping.installation_id = session.installation_id AND mapping.id = session.mapping_id \
                 JOIN users user_account ON user_account.installation_id = session.installation_id AND user_account.id = session.user_id \
                 WHERE session.installation_id = ?1 AND session.mapping_id = ?2 AND session.session_hash = ?3 \
                 AND session.revoked_at IS NULL AND session.expires_at >= ?4 AND user_account.disabled = 0 \
                 AND (user_account.system_admin <> 0 OR EXISTS (SELECT 1 FROM organization_memberships membership \
                      WHERE membership.installation_id = session.installation_id \
                      AND membership.organization_id = mapping.organization_id AND membership.user_id = user_account.id))",
            ).bind(installation_id.as_str()).bind(mapping_id.to_string()).bind(session_hash).bind(now).fetch_optional(pool).await?,
            Self::Postgres { pool, installation_id } => sqlx::query_scalar(
                "SELECT 1::BIGINT FROM workspace_port_mapping_sessions session \
                 JOIN workspace_port_mappings mapping ON mapping.installation_id = session.installation_id AND mapping.id = session.mapping_id \
                 JOIN users user_account ON user_account.installation_id = session.installation_id AND user_account.id = session.user_id \
                 WHERE session.installation_id = $1 AND session.mapping_id = $2 AND session.session_hash = $3 \
                 AND session.revoked_at IS NULL AND session.expires_at >= $4 AND user_account.disabled = 0 \
                 AND (user_account.system_admin <> 0 OR EXISTS (SELECT 1 FROM organization_memberships membership \
                      WHERE membership.installation_id = session.installation_id \
                      AND membership.organization_id = mapping.organization_id AND membership.user_id = user_account.id))",
            ).bind(installation_id.as_str()).bind(mapping_id.to_string()).bind(session_hash).bind(now).fetch_optional(pool).await?,
        };
        Ok(found.is_some())
    }

    async fn port_mapping_by_workspace_port(
        &self,
        workspace_id: Uuid,
        internal_port: u16,
    ) -> Result<PortMapping, StorageError> {
        match self {
            Self::Sqlite { pool, installation_id } => decode(sqlx::query("SELECT id, organization_id, workspace_id, internal_port, display_name, subdomain, created_by, created_at FROM workspace_port_mappings WHERE installation_id = ?1 AND workspace_id = ?2 AND internal_port = ?3").bind(installation_id.as_str()).bind(workspace_id.to_string()).bind(i64::from(internal_port)).fetch_optional(pool).await?.ok_or(StorageError::PortMappingNotFound)?),
            Self::Postgres { pool, installation_id } => decode(sqlx::query("SELECT id, organization_id, workspace_id, internal_port, display_name, subdomain, created_by, created_at FROM workspace_port_mappings WHERE installation_id = $1 AND workspace_id = $2 AND internal_port = $3").bind(installation_id.as_str()).bind(workspace_id.to_string()).bind(i64::from(internal_port)).fetch_optional(pool).await?.ok_or(StorageError::PortMappingNotFound)?),
        }
    }
}

// The route job is durable in the same transaction as the declaration. It reads
// the latest mappings after acquiring the workspace lease, so no payload snapshot
// or stale-generation replay can restore a deleted route.
async fn enqueue_routes_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    workspace_id: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, '{}', 'pending', ?5, NULL, NULL, 0, ?5, ?5)")
        .bind(Uuid::now_v7().to_string())
        .bind(installation)
        .bind(PORT_MAPPING_RECONCILE_JOB)
        .bind(workspace_id.to_string())
        .bind(now)
        .execute(connection)
        .await?;
    Ok(())
}

async fn enqueue_routes_postgres(
    connection: &mut PgConnection,
    installation: &str,
    workspace_id: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES ($1, $2, $3, $4, '{}', 'pending', $5, NULL, NULL, 0, $5, $5)")
        .bind(Uuid::now_v7().to_string())
        .bind(installation)
        .bind(PORT_MAPPING_RECONCILE_JOB)
        .bind(workspace_id.to_string())
        .bind(now)
        .execute(connection)
        .await?;
    Ok(())
}

fn decode<R: sqlx::Row>(row: R) -> Result<PortMapping, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
    i64: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
{
    Ok(PortMapping {
        id: Uuid::parse_str(&row.try_get::<String, _>("id")?)?,
        organization_id: Uuid::parse_str(&row.try_get::<String, _>("organization_id")?)?,
        workspace_id: Uuid::parse_str(&row.try_get::<String, _>("workspace_id")?)?,
        internal_port: u16::try_from(row.try_get::<i64, _>("internal_port")?)
            .map_err(|_| StorageError::InvalidPortMappingPort)?,
        display_name: row.try_get("display_name")?,
        subdomain: row.try_get("subdomain")?,
        created_by: Uuid::parse_str(&row.try_get::<String, _>("created_by")?)?,
        created_at: row.try_get("created_at")?,
    })
}

pub fn hash_secret(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn validate_display_name(value: Option<&str>) -> Result<Option<String>, StorageError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if value.chars().count() > 80 || value.chars().any(|character| character.is_control()) {
        return Err(StorageError::InvalidPortMappingDisplayName);
    }
    Ok(Some(value.to_owned()))
}

fn normalize_subdomain(value: &str) -> Result<String, StorageError> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty()
        || value.len() > 61
        || !value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err(StorageError::InvalidPortMappingSubdomain);
    }
    Ok(value)
}

fn random_subdomain() -> Result<String, StorageError> {
    let mut random = [0_u8; 4];
    getrandom::fill(&mut random).map_err(|_| StorageError::RandomSource)?;
    Ok(format!("{:08x}", u32::from_be_bytes(random)))
}

fn subdomain_write_error(error: sqlx::Error) -> StorageError {
    if is_subdomain_conflict(&error) {
        StorageError::PortMappingSubdomainConflict
    } else {
        StorageError::Database(error)
    }
}

fn is_subdomain_conflict(error: &sqlx::Error) -> bool {
    error.as_database_error().is_some_and(|error| {
        error.is_unique_violation()
            && (error.constraint() == Some("workspace_port_mappings_subdomain_idx")
                || error
                    .message()
                    .contains("workspace_port_mappings.subdomain"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_application_ports_and_rejects_platform_ports() {
        assert!(validate_http_port(80).is_ok());
        assert!(validate_http_port(3000).is_ok());
        assert!(validate_http_port(2222).is_err());
        assert!(validate_http_port(7681).is_err());
        assert!(validate_http_port(8080).is_err());
        assert!(validate_http_port(3389).is_err());
    }

    #[test]
    fn display_name_limit_counts_unicode_characters_instead_of_utf8_bytes() {
        for character in ["中", "a", "🦀"] {
            let name = character.repeat(80);
            assert_eq!(validate_display_name(Some(&name)).unwrap(), Some(name));
            assert!(matches!(
                validate_display_name(Some(&character.repeat(81))),
                Err(StorageError::InvalidPortMappingDisplayName)
            ));
        }
    }

    #[test]
    fn normalizes_display_names_without_accepting_controls() {
        assert_eq!(
            validate_display_name(Some("  frontend  ")).unwrap(),
            Some("frontend".to_owned())
        );
        assert_eq!(validate_display_name(Some("  ")).unwrap(), None);
        assert!(validate_display_name(Some("bad\nname")).is_err());
    }
}
