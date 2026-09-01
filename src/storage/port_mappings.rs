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
use utoipa::ToSchema;
use uuid::Uuid;

use super::{Database, StorageError};

/// HTTP ports which a workspace is allowed to publish through the authenticated
/// gateway.  Port 80 is deliberately allowed: it is a *container* port behind
/// a ClusterIP service, never a host port or NodePort.
pub fn validate_http_port(port: u16) -> Result<(), StorageError> {
    let allowed = port == 80 || port == 443 || (1024..=65535).contains(&port);
    // These are control-plane / workspace platform listeners, not application
    // ports.  Keeping the deny-list here makes every caller enforce it.
    let reserved = matches!(port, 22 | 2222 | 7681 | 8080 | 8081 | 8443);
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
    pub created_by: Uuid,
    pub created_at: i64,
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
    /// Inserts a mapping exactly once per workspace/port.  Concurrent requests
    /// (including requests reaching different control-plane replicas) converge
    /// on the same row rather than allocating duplicate public resources.
    pub async fn create_port_mapping(
        &self,
        organization_id: Uuid,
        workspace_id: Uuid,
        internal_port: u16,
        display_name: Option<&str>,
        created_by: Uuid,
        created_at: i64,
    ) -> Result<PortMapping, StorageError> {
        validate_http_port(internal_port)?;
        let display_name = validate_display_name(display_name)?;
        let id = Uuid::now_v7();
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO workspace_port_mappings (id, installation_id, organization_id, workspace_id, internal_port, display_name, created_by, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ON CONFLICT (installation_id, workspace_id, internal_port) DO NOTHING")
                    .bind(id.to_string()).bind(installation_id.as_str()).bind(organization_id.to_string())
                    .bind(workspace_id.to_string()).bind(i64::from(internal_port)).bind(display_name).bind(created_by.to_string()).bind(created_at)
                    .execute(pool).await?;
                self.port_mapping_by_workspace_port(workspace_id, internal_port)
                    .await
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO workspace_port_mappings (id, installation_id, organization_id, workspace_id, internal_port, display_name, created_by, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (installation_id, workspace_id, internal_port) DO NOTHING")
                    .bind(id.to_string()).bind(installation_id.as_str()).bind(organization_id.to_string())
                    .bind(workspace_id.to_string()).bind(i64::from(internal_port)).bind(display_name).bind(created_by.to_string()).bind(created_at)
                    .execute(pool).await?;
                self.port_mapping_by_workspace_port(workspace_id, internal_port)
                    .await
            }
        }
    }

    pub async fn list_port_mappings(
        &self,
        workspace_id: Uuid,
    ) -> Result<Vec<PortMapping>, StorageError> {
        let sql_sqlite = "SELECT id, organization_id, workspace_id, internal_port, display_name, created_by, created_at FROM workspace_port_mappings WHERE installation_id = ?1 AND workspace_id = ?2 ORDER BY internal_port";
        let sql_postgres = "SELECT id, organization_id, workspace_id, internal_port, display_name, created_by, created_at FROM workspace_port_mappings WHERE installation_id = $1 AND workspace_id = $2 ORDER BY internal_port";
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
    ) -> Result<bool, StorageError> {
        let deleted = match self {
            Self::Sqlite { pool, installation_id } => sqlx::query("DELETE FROM workspace_port_mappings WHERE installation_id = ?1 AND workspace_id = ?2 AND id = ?3").bind(installation_id.as_str()).bind(workspace_id.to_string()).bind(mapping_id.to_string()).execute(pool).await?.rows_affected() == 1,
            Self::Postgres { pool, installation_id } => sqlx::query("DELETE FROM workspace_port_mappings WHERE installation_id = $1 AND workspace_id = $2 AND id = $3").bind(installation_id.as_str()).bind(workspace_id.to_string()).bind(mapping_id.to_string()).execute(pool).await?.rows_affected() == 1,
        };
        Ok(deleted)
    }

    pub async fn get_port_mapping(&self, mapping_id: Uuid) -> Result<PortMapping, StorageError> {
        match self {
            Self::Sqlite { pool, installation_id } => decode(sqlx::query("SELECT id, organization_id, workspace_id, internal_port, display_name, created_by, created_at FROM workspace_port_mappings WHERE installation_id = ?1 AND id = ?2").bind(installation_id.as_str()).bind(mapping_id.to_string()).fetch_optional(pool).await?.ok_or(StorageError::PortMappingNotFound)?),
            Self::Postgres { pool, installation_id } => decode(sqlx::query("SELECT id, organization_id, workspace_id, internal_port, display_name, created_by, created_at FROM workspace_port_mappings WHERE installation_id = $1 AND id = $2").bind(installation_id.as_str()).bind(mapping_id.to_string()).fetch_optional(pool).await?.ok_or(StorageError::PortMappingNotFound)?),
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
            Self::Sqlite { pool, installation_id } => sqlx::query_scalar("SELECT 1 FROM workspace_port_mapping_sessions WHERE installation_id = ?1 AND mapping_id = ?2 AND session_hash = ?3 AND revoked_at IS NULL AND expires_at >= ?4").bind(installation_id.as_str()).bind(mapping_id.to_string()).bind(session_hash).bind(now).fetch_optional(pool).await?,
            Self::Postgres { pool, installation_id } => sqlx::query_scalar("SELECT 1 FROM workspace_port_mapping_sessions WHERE installation_id = $1 AND mapping_id = $2 AND session_hash = $3 AND revoked_at IS NULL AND expires_at >= $4").bind(installation_id.as_str()).bind(mapping_id.to_string()).bind(session_hash).bind(now).fetch_optional(pool).await?,
        };
        Ok(found.is_some())
    }

    async fn port_mapping_by_workspace_port(
        &self,
        workspace_id: Uuid,
        internal_port: u16,
    ) -> Result<PortMapping, StorageError> {
        match self {
            Self::Sqlite { pool, installation_id } => decode(sqlx::query("SELECT id, organization_id, workspace_id, internal_port, display_name, created_by, created_at FROM workspace_port_mappings WHERE installation_id = ?1 AND workspace_id = ?2 AND internal_port = ?3").bind(installation_id.as_str()).bind(workspace_id.to_string()).bind(i64::from(internal_port)).fetch_optional(pool).await?.ok_or(StorageError::PortMappingNotFound)?),
            Self::Postgres { pool, installation_id } => decode(sqlx::query("SELECT id, organization_id, workspace_id, internal_port, display_name, created_by, created_at FROM workspace_port_mappings WHERE installation_id = $1 AND workspace_id = $2 AND internal_port = $3").bind(installation_id.as_str()).bind(workspace_id.to_string()).bind(i64::from(internal_port)).fetch_optional(pool).await?.ok_or(StorageError::PortMappingNotFound)?),
        }
    }
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
    if value.len() > 80 || value.chars().any(|character| character.is_control()) {
        return Err(StorageError::InvalidPortMappingDisplayName);
    }
    Ok(Some(value.to_owned()))
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
