use std::fmt;

use base64::{Engine, engine::general_purpose::STANDARD};
use rand_core::OsRng;
use serde::Serialize;
use sqlx::{Row, postgres::PgRow, sqlite::SqliteRow};
use ssh_key::{Algorithm, HashAlg, LineEnding, PrivateKey};
use utoipa::ToSchema;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::crypto::{EncryptedEnvelope, EnvelopeCipher};

use super::{Database, StorageError};

pub struct WorkspaceSshIdentity {
    pub public: WorkspaceSshPublicIdentity,
    pub private_key: Zeroizing<String>,
}

impl fmt::Debug for WorkspaceSshIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkspaceSshIdentity")
            .field("public", &self.public)
            .field("private_key", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WorkspaceSshPublicIdentity {
    pub algorithm: &'static str,
    pub public_key: String,
    pub fingerprint: String,
}

impl Database {
    pub async fn ensure_workspace_ssh_identity(
        &self,
        cipher: &EnvelopeCipher,
        workspace_id: Uuid,
        now: i64,
    ) -> Result<WorkspaceSshIdentity, StorageError> {
        if let Some(identity) = self
            .load_workspace_ssh_identity(cipher, workspace_id)
            .await?
        {
            return Ok(identity);
        }
        let mut private = PrivateKey::random(&mut OsRng, Algorithm::Ed25519)
            .map_err(|_| StorageError::InvalidSshIdentity)?;
        private.set_comment(format!("mwc-workspace-{workspace_id}"));
        let private_key = private
            .to_openssh(LineEnding::LF)
            .map_err(|_| StorageError::InvalidSshIdentity)?;
        let public_key = private
            .public_key()
            .to_openssh()
            .map_err(|_| StorageError::InvalidSshIdentity)?;
        let fingerprint = private.fingerprint(HashAlg::Sha256).to_string();
        let aad = aad(self.installation_id().as_str(), workspace_id);
        let envelope = cipher.encrypt(private_key.as_bytes(), aad.as_bytes())?;
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO workspace_ssh_identities (installation_id, workspace_id, public_key, fingerprint, ciphertext, value_nonce, wrapped_data_key, key_nonce, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT (installation_id, workspace_id) DO NOTHING")
                .bind(installation_id.as_str()).bind(workspace_id.to_string()).bind(public_key).bind(fingerprint).bind(STANDARD.encode(envelope.ciphertext)).bind(STANDARD.encode(envelope.value_nonce)).bind(STANDARD.encode(envelope.wrapped_data_key)).bind(STANDARD.encode(envelope.key_nonce)).bind(now).execute(pool).await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO workspace_ssh_identities (installation_id, workspace_id, public_key, fingerprint, ciphertext, value_nonce, wrapped_data_key, key_nonce, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) ON CONFLICT (installation_id, workspace_id) DO NOTHING")
                .bind(installation_id.as_str()).bind(workspace_id.to_string()).bind(public_key).bind(fingerprint).bind(STANDARD.encode(envelope.ciphertext)).bind(STANDARD.encode(envelope.value_nonce)).bind(STANDARD.encode(envelope.wrapped_data_key)).bind(STANDARD.encode(envelope.key_nonce)).bind(now).execute(pool).await?;
            }
        }
        self.load_workspace_ssh_identity(cipher, workspace_id)
            .await?
            .ok_or(StorageError::InvalidSshIdentity)
    }

    pub async fn workspace_ssh_public_identity(
        &self,
        workspace_id: Uuid,
    ) -> Result<Option<WorkspaceSshPublicIdentity>, StorageError> {
        let row = match self {
            Self::Sqlite { pool, installation_id } => sqlx::query("SELECT public_key, fingerprint FROM workspace_ssh_identities WHERE installation_id = ?1 AND workspace_id = ?2").bind(installation_id.as_str()).bind(workspace_id.to_string()).fetch_optional(pool).await?.map(SshRow::Sqlite),
            Self::Postgres { pool, installation_id } => sqlx::query("SELECT public_key, fingerprint FROM workspace_ssh_identities WHERE installation_id = $1 AND workspace_id = $2").bind(installation_id.as_str()).bind(workspace_id.to_string()).fetch_optional(pool).await?.map(SshRow::Postgres),
        };
        row.map(decode_public).transpose()
    }

    async fn load_workspace_ssh_identity(
        &self,
        cipher: &EnvelopeCipher,
        workspace_id: Uuid,
    ) -> Result<Option<WorkspaceSshIdentity>, StorageError> {
        let row = match self {
            Self::Sqlite { pool, installation_id } => sqlx::query("SELECT public_key, fingerprint, ciphertext, value_nonce, wrapped_data_key, key_nonce FROM workspace_ssh_identities WHERE installation_id = ?1 AND workspace_id = ?2").bind(installation_id.as_str()).bind(workspace_id.to_string()).fetch_optional(pool).await?.map(SshRow::Sqlite),
            Self::Postgres { pool, installation_id } => sqlx::query("SELECT public_key, fingerprint, ciphertext, value_nonce, wrapped_data_key, key_nonce FROM workspace_ssh_identities WHERE installation_id = $1 AND workspace_id = $2").bind(installation_id.as_str()).bind(workspace_id.to_string()).fetch_optional(pool).await?.map(SshRow::Postgres),
        };
        let Some(row) = row else {
            return Ok(None);
        };
        let (public, envelope) = decode_identity_row(row)?;
        let private = cipher.decrypt(
            &envelope,
            aad(self.installation_id().as_str(), workspace_id).as_bytes(),
        )?;
        let private_key = Zeroizing::new(
            String::from_utf8(private.to_vec()).map_err(|_| StorageError::InvalidSshIdentity)?,
        );
        Ok(Some(WorkspaceSshIdentity {
            public,
            private_key,
        }))
    }
}

enum SshRow {
    Sqlite(SqliteRow),
    Postgres(PgRow),
}
fn decode_public(row: SshRow) -> Result<WorkspaceSshPublicIdentity, StorageError> {
    match row {
        SshRow::Sqlite(row) => decode_public_row(&row),
        SshRow::Postgres(row) => decode_public_row(&row),
    }
}
fn decode_identity_row(
    row: SshRow,
) -> Result<(WorkspaceSshPublicIdentity, EncryptedEnvelope), StorageError> {
    match row {
        SshRow::Sqlite(row) => decode_full_row(&row),
        SshRow::Postgres(row) => decode_full_row(&row),
    }
}
fn decode_public_row<R: Row>(row: &R) -> Result<WorkspaceSshPublicIdentity, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    Ok(WorkspaceSshPublicIdentity {
        algorithm: "ssh-ed25519",
        public_key: row.try_get("public_key")?,
        fingerprint: row.try_get("fingerprint")?,
    })
}
fn decode_full_row<R: Row>(
    row: &R,
) -> Result<(WorkspaceSshPublicIdentity, EncryptedEnvelope), StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    Ok((
        decode_public_row(row)?,
        EncryptedEnvelope {
            ciphertext: decode(row, "ciphertext")?,
            value_nonce: decode_array(row, "value_nonce")?,
            wrapped_data_key: decode(row, "wrapped_data_key")?,
            key_nonce: decode_array(row, "key_nonce")?,
        },
    ))
}

fn decode<R: Row>(row: &R, column: &str) -> Result<Vec<u8>, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    STANDARD
        .decode(row.try_get::<String, _>(column)?)
        .map_err(|_| StorageError::InvalidSshIdentity)
}
fn decode_array<R: Row, const N: usize>(row: &R, column: &str) -> Result<[u8; N], StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    decode(row, column)?
        .try_into()
        .map_err(|_| StorageError::InvalidSshIdentity)
}
fn aad(installation: &str, workspace_id: Uuid) -> String {
    format!("{installation}:workspace-ssh:{workspace_id}:v1")
}
