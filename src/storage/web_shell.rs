use std::fmt;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{Database, StorageError};

const TICKET_BYTES: usize = 32;

#[derive(Clone, Serialize, ToSchema)]
pub struct IssuedWebShellTicket {
    pub ticket: String,
    pub workspace_id: Uuid,
    pub expires_at: i64,
}

impl fmt::Debug for IssuedWebShellTicket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IssuedWebShellTicket")
            .field("ticket", &"[REDACTED]")
            .field("workspace_id", &self.workspace_id)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebShellIdentity {
    pub organization_id: Uuid,
    pub workspace_id: Uuid,
    pub user_id: Uuid,
}

impl Database {
    pub async fn issue_web_shell_ticket(
        &self,
        organization_id: Uuid,
        workspace_id: Uuid,
        user_id: Uuid,
        now: i64,
        ttl_seconds: i64,
    ) -> Result<IssuedWebShellTicket, StorageError> {
        if !(1..=300).contains(&ttl_seconds) {
            return Err(StorageError::InvalidTicketTtl);
        }
        let expires_at = now
            .checked_add(ttl_seconds)
            .ok_or(StorageError::InvalidTicketTtl)?;
        let mut bytes = [0_u8; TICKET_BYTES];
        getrandom::fill(&mut bytes).map_err(|_| StorageError::RandomSource)?;
        let ticket = URL_SAFE_NO_PAD.encode(bytes);
        let ticket_hash = hash_ticket(&ticket);
        let id = Uuid::now_v7();
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO web_shell_tickets (id, installation_id, ticket_hash, organization_id, workspace_id, user_id, expires_at, consumed_at, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8)")
                    .bind(id.to_string()).bind(installation_id.as_str()).bind(ticket_hash)
                    .bind(organization_id.to_string()).bind(workspace_id.to_string())
                    .bind(user_id.to_string()).bind(expires_at).bind(now).execute(pool).await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO web_shell_tickets (id, installation_id, ticket_hash, organization_id, workspace_id, user_id, expires_at, consumed_at, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, NULL, $8)")
                    .bind(id.to_string()).bind(installation_id.as_str()).bind(ticket_hash)
                    .bind(organization_id.to_string()).bind(workspace_id.to_string())
                    .bind(user_id.to_string()).bind(expires_at).bind(now).execute(pool).await?;
            }
        }
        Ok(IssuedWebShellTicket {
            ticket,
            workspace_id,
            expires_at,
        })
    }

    pub async fn consume_web_shell_ticket(
        &self,
        ticket: &str,
        workspace_id: Uuid,
        now: i64,
    ) -> Result<Option<WebShellIdentity>, StorageError> {
        if ticket.len() < 32 {
            return Ok(None);
        }
        let ticket_hash = hash_ticket(ticket);
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let row = sqlx::query("UPDATE web_shell_tickets SET consumed_at = ?1 WHERE installation_id = ?2 AND ticket_hash = ?3 AND workspace_id = ?4 AND consumed_at IS NULL AND expires_at >= ?1 RETURNING organization_id, workspace_id, user_id")
                .bind(now).bind(installation_id.as_str()).bind(ticket_hash)
                .bind(workspace_id.to_string()).fetch_optional(pool).await?;
                row.map(decode_identity).transpose()
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let row = sqlx::query("UPDATE web_shell_tickets SET consumed_at = $1 WHERE installation_id = $2 AND ticket_hash = $3 AND workspace_id = $4 AND consumed_at IS NULL AND expires_at >= $1 RETURNING organization_id, workspace_id, user_id")
                .bind(now).bind(installation_id.as_str()).bind(ticket_hash)
                .bind(workspace_id.to_string()).fetch_optional(pool).await?;
                row.map(decode_identity).transpose()
            }
        }
    }
}

fn decode_identity<R: sqlx::Row>(row: R) -> Result<WebShellIdentity, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
{
    Ok(WebShellIdentity {
        organization_id: Uuid::parse_str(&row.try_get::<String, _>("organization_id")?)?,
        workspace_id: Uuid::parse_str(&row.try_get::<String, _>("workspace_id")?)?,
        user_id: Uuid::parse_str(&row.try_get::<String, _>("user_id")?)?,
    })
}

fn hash_ticket(ticket: &str) -> String {
    format!("{:x}", Sha256::digest(ticket.as_bytes()))
}
