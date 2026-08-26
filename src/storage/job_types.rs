use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Row, postgres::PgRow, sqlite::SqliteRow};
use uuid::Uuid;

use super::StorageError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewJob {
    pub kind: String,
    pub workspace_id: Option<Uuid>,
    pub payload: Value,
    pub available_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClaimedJob {
    pub id: Uuid,
    pub kind: String,
    pub workspace_id: Option<Uuid>,
    pub payload: Value,
    pub attempts: i64,
    pub lease_expires_at: i64,
}

pub(super) fn decode_sqlite_job(row: SqliteRow) -> Result<ClaimedJob, StorageError> {
    decode_job_fields(
        row.try_get("id")?,
        row.try_get("kind")?,
        row.try_get("workspace_id")?,
        row.try_get("payload_json")?,
        row.try_get("attempts")?,
        row.try_get("lease_expires_at")?,
    )
}

pub(super) fn decode_postgres_job(row: PgRow) -> Result<ClaimedJob, StorageError> {
    decode_job_fields(
        row.try_get("id")?,
        row.try_get("kind")?,
        row.try_get("workspace_id")?,
        row.try_get("payload_json")?,
        row.try_get("attempts")?,
        row.try_get("lease_expires_at")?,
    )
}

fn decode_job_fields(
    id: String,
    kind: String,
    workspace_id: Option<String>,
    payload_json: String,
    attempts: i64,
    lease_expires_at: i64,
) -> Result<ClaimedJob, StorageError> {
    Ok(ClaimedJob {
        id: Uuid::parse_str(&id)?,
        kind,
        workspace_id: workspace_id
            .map(|value| Uuid::parse_str(&value))
            .transpose()?,
        payload: serde_json::from_str(&payload_json)?,
        attempts,
        lease_expires_at,
    })
}
