use std::{fmt, net::IpAddr};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, Row, SqliteConnection, postgres::PgRow, sqlite::SqliteRow};
use url::Url;
use utoipa::ToSchema;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::crypto::{EncryptedEnvelope, EnvelopeCipher};

use super::{Database, EventRecord, StorageError};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateWebhookSubscription {
    pub organization_id: Uuid,
    pub url: String,
    pub event_prefix: String,
    pub signing_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WebhookSubscriptionSummary {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub url: String,
    pub event_prefix: String,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

pub struct WebhookDelivery {
    pub subscription: WebhookSubscriptionSummary,
    pub event: EventRecord,
    pub signing_secret: Zeroizing<Vec<u8>>,
}

impl fmt::Debug for WebhookDelivery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebhookDelivery")
            .field("subscription", &self.subscription)
            .field("event", &self.event)
            .field("signing_secret", &"[REDACTED]")
            .finish()
    }
}

impl Database {
    pub async fn create_webhook_subscription(
        &self,
        cipher: &EnvelopeCipher,
        command: CreateWebhookSubscription,
        actor: Uuid,
        now: i64,
    ) -> Result<WebhookSubscriptionSummary, StorageError> {
        validate(&command)?;
        let id = Uuid::now_v7();
        let aad = webhook_aad(self.installation_id().as_str(), id);
        let envelope = cipher.encrypt(command.signing_secret.as_bytes(), aad.as_bytes())?;
        let summary = WebhookSubscriptionSummary {
            id,
            organization_id: command.organization_id,
            url: command.url,
            event_prefix: command.event_prefix,
            enabled: true,
            created_at: now,
            updated_at: now,
        };
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO webhook_subscriptions (id, installation_id, organization_id, url, event_prefix, ciphertext, value_nonce, wrapped_data_key, key_nonce, enabled, created_by, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10, ?11, ?11)")
                .bind(id.to_string()).bind(installation_id.as_str()).bind(command.organization_id.to_string()).bind(&summary.url).bind(&summary.event_prefix).bind(STANDARD.encode(envelope.ciphertext)).bind(STANDARD.encode(envelope.value_nonce)).bind(STANDARD.encode(envelope.wrapped_data_key)).bind(STANDARD.encode(envelope.key_nonce)).bind(actor.to_string()).bind(now).execute(pool).await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO webhook_subscriptions (id, installation_id, organization_id, url, event_prefix, ciphertext, value_nonce, wrapped_data_key, key_nonce, enabled, created_by, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 1, $10, $11, $11)")
                .bind(id.to_string()).bind(installation_id.as_str()).bind(command.organization_id.to_string()).bind(&summary.url).bind(&summary.event_prefix).bind(STANDARD.encode(envelope.ciphertext)).bind(STANDARD.encode(envelope.value_nonce)).bind(STANDARD.encode(envelope.wrapped_data_key)).bind(STANDARD.encode(envelope.key_nonce)).bind(actor.to_string()).bind(now).execute(pool).await?;
            }
        }
        Ok(summary)
    }

    pub async fn list_webhook_subscriptions(
        &self,
        organization_id: Uuid,
    ) -> Result<Vec<WebhookSubscriptionSummary>, StorageError> {
        match self {
            Self::Sqlite { pool, installation_id } => sqlx::query("SELECT id, organization_id, url, event_prefix, enabled, created_at, updated_at FROM webhook_subscriptions WHERE installation_id = ?1 AND organization_id = ?2 ORDER BY created_at, id").bind(installation_id.as_str()).bind(organization_id.to_string()).fetch_all(pool).await?.into_iter().map(decode_sqlite_summary).collect(),
            Self::Postgres { pool, installation_id } => sqlx::query("SELECT id, organization_id, url, event_prefix, enabled, created_at, updated_at FROM webhook_subscriptions WHERE installation_id = $1 AND organization_id = $2 ORDER BY created_at, id").bind(installation_id.as_str()).bind(organization_id.to_string()).fetch_all(pool).await?.into_iter().map(decode_postgres_summary).collect(),
        }
    }

    pub async fn load_webhook_delivery(
        &self,
        cipher: &EnvelopeCipher,
        subscription_id: Uuid,
        event_id: Uuid,
    ) -> Result<WebhookDelivery, StorageError> {
        let row = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(&delivery_sql("?1", "?2", "?3"))
                .bind(installation_id.as_str())
                .bind(subscription_id.to_string())
                .bind(event_id.to_string())
                .fetch_optional(pool)
                .await?
                .map(DeliveryRow::Sqlite),
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(&delivery_sql("$1", "$2", "$3"))
                .bind(installation_id.as_str())
                .bind(subscription_id.to_string())
                .bind(event_id.to_string())
                .fetch_optional(pool)
                .await?
                .map(DeliveryRow::Postgres),
        }
        .ok_or(StorageError::WebhookNotFound)?;
        decode_delivery(self.installation_id().as_str(), cipher, row)
    }
}

pub(super) async fn enqueue_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    organization_id: Uuid,
    workspace_id: Uuid,
    event_id: Uuid,
    kind: &str,
    now: i64,
) -> Result<(), StorageError> {
    let rows = sqlx::query("SELECT id FROM webhook_subscriptions WHERE installation_id = ?1 AND organization_id = ?2 AND enabled = 1 AND ?3 LIKE event_prefix || '%'").bind(installation).bind(organization_id.to_string()).bind(kind).fetch_all(&mut *connection).await?;
    for row in rows {
        insert_delivery_job_sqlite(
            connection,
            installation,
            row.try_get::<String, _>("id")?,
            workspace_id,
            event_id,
            now,
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn enqueue_postgres(
    connection: &mut PgConnection,
    installation: &str,
    organization_id: Uuid,
    workspace_id: Uuid,
    event_id: Uuid,
    kind: &str,
    now: i64,
) -> Result<(), StorageError> {
    let rows = sqlx::query("SELECT id FROM webhook_subscriptions WHERE installation_id = $1 AND organization_id = $2 AND enabled = 1 AND $3 LIKE event_prefix || '%'").bind(installation).bind(organization_id.to_string()).bind(kind).fetch_all(&mut *connection).await?;
    for row in rows {
        insert_delivery_job_postgres(
            connection,
            installation,
            row.try_get::<String, _>("id")?,
            workspace_id,
            event_id,
            now,
        )
        .await?;
    }
    Ok(())
}

async fn insert_delivery_job_sqlite(
    connection: &mut SqliteConnection,
    installation: &str,
    subscription_id: String,
    workspace_id: Uuid,
    event_id: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES (?1, ?2, 'deliver_webhook', ?3, ?4, 'pending', ?5, NULL, NULL, 0, ?5, ?5)").bind(Uuid::now_v7().to_string()).bind(installation).bind(workspace_id.to_string()).bind(serde_json::json!({"subscription_id": subscription_id, "event_id": event_id}).to_string()).bind(now).execute(connection).await?;
    Ok(())
}
async fn insert_delivery_job_postgres(
    connection: &mut PgConnection,
    installation: &str,
    subscription_id: String,
    workspace_id: Uuid,
    event_id: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES ($1, $2, 'deliver_webhook', $3, $4, 'pending', $5, NULL, NULL, 0, $5, $5)").bind(Uuid::now_v7().to_string()).bind(installation).bind(workspace_id.to_string()).bind(serde_json::json!({"subscription_id": subscription_id, "event_id": event_id}).to_string()).bind(now).execute(connection).await?;
    Ok(())
}

fn validate(command: &CreateWebhookSubscription) -> Result<(), StorageError> {
    let url = Url::parse(&command.url).map_err(|_| StorageError::InvalidWebhook)?;
    if url.scheme() != "https"
        || url.username() != ""
        || url.password().is_some()
        || command.event_prefix.is_empty()
        || command.event_prefix.len() > 120
        || command.signing_secret.len() < 32
    {
        return Err(StorageError::InvalidWebhook);
    }
    let host = url.host_str().ok_or(StorageError::InvalidWebhook)?;
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".local") {
        return Err(StorageError::InvalidWebhook);
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        let unsafe_address = match ip {
            IpAddr::V4(ip) => {
                ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified()
            }
            IpAddr::V6(ip) => {
                ip.is_loopback()
                    || ip.is_unique_local()
                    || ip.is_unicast_link_local()
                    || ip.is_unspecified()
            }
        };
        if unsafe_address {
            return Err(StorageError::InvalidWebhook);
        }
    }
    Ok(())
}

fn delivery_sql(installation: &str, subscription: &str, event: &str) -> String {
    format!(
        "SELECT s.id, s.organization_id, s.url, s.event_prefix, s.enabled, s.created_at, s.updated_at, s.ciphertext, s.value_nonce, s.wrapped_data_key, s.key_nonce, e.id event_id, e.workspace_id, e.kind, e.payload_json, e.created_at event_created_at FROM webhook_subscriptions s JOIN events e ON e.installation_id = s.installation_id AND e.organization_id = s.organization_id WHERE s.installation_id = {installation} AND s.id = {subscription} AND e.id = {event} AND s.enabled = 1"
    )
}
enum DeliveryRow {
    Sqlite(SqliteRow),
    Postgres(PgRow),
}
fn decode_delivery(
    installation: &str,
    cipher: &EnvelopeCipher,
    row: DeliveryRow,
) -> Result<WebhookDelivery, StorageError> {
    match row {
        DeliveryRow::Sqlite(row) => decode_delivery_row(installation, cipher, &row),
        DeliveryRow::Postgres(row) => decode_delivery_row(installation, cipher, &row),
    }
}
fn decode_delivery_row<R: Row>(
    installation: &str,
    cipher: &EnvelopeCipher,
    row: &R,
) -> Result<WebhookDelivery, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    let subscription = decode_summary_row(row)?;
    let event_id = Uuid::parse_str(&row.try_get::<String, _>("event_id")?)?;
    let workspace: Option<String> = row.try_get("workspace_id")?;
    let envelope = EncryptedEnvelope {
        ciphertext: decode_base64(row, "ciphertext")?,
        value_nonce: decode_array(row, "value_nonce")?,
        wrapped_data_key: decode_base64(row, "wrapped_data_key")?,
        key_nonce: decode_array(row, "key_nonce")?,
    };
    let signing_secret = cipher.decrypt(
        &envelope,
        webhook_aad(installation, subscription.id).as_bytes(),
    )?;
    Ok(WebhookDelivery {
        subscription,
        event: EventRecord {
            id: event_id,
            organization_id: Uuid::parse_str(&row.try_get::<String, _>("organization_id")?)?,
            workspace_id: workspace.map(|id| Uuid::parse_str(&id)).transpose()?,
            kind: row.try_get("kind")?,
            payload: serde_json::from_str(&row.try_get::<String, _>("payload_json")?)?,
            created_at: row.try_get("event_created_at")?,
        },
        signing_secret,
    })
}
fn decode_sqlite_summary(row: SqliteRow) -> Result<WebhookSubscriptionSummary, StorageError> {
    decode_summary_row(&row)
}
fn decode_postgres_summary(row: PgRow) -> Result<WebhookSubscriptionSummary, StorageError> {
    decode_summary_row(&row)
}
fn decode_summary_row<R: Row>(row: &R) -> Result<WebhookSubscriptionSummary, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    Ok(WebhookSubscriptionSummary {
        id: Uuid::parse_str(&row.try_get::<String, _>("id")?)?,
        organization_id: Uuid::parse_str(&row.try_get::<String, _>("organization_id")?)?,
        url: row.try_get("url")?,
        event_prefix: row.try_get("event_prefix")?,
        enabled: row.try_get::<i64, _>("enabled")? != 0,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}
fn decode_base64<R: Row>(row: &R, column: &str) -> Result<Vec<u8>, StorageError>
where
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
{
    STANDARD
        .decode(row.try_get::<String, _>(column)?)
        .map_err(|_| StorageError::InvalidEncryptedInjection)
}
fn decode_array<R: Row, const N: usize>(row: &R, column: &str) -> Result<[u8; N], StorageError>
where
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
{
    decode_base64(row, column)?
        .try_into()
        .map_err(|_| StorageError::InvalidEncryptedInjection)
}
fn webhook_aad(installation: &str, id: Uuid) -> String {
    format!("{installation}:webhook:{id}:v1")
}
