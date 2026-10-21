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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParkedWorkspaceJob {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// Preserve the exact stored representation, including the park nonce, for wake CAS.
    pub payload_json: String,
    pub until: i64,
    pub generation: u64,
    pub reconciled_at: i64,
    pub desired_replicas: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParkedWorkspaceJobsPage {
    pub jobs: Vec<ParkedWorkspaceJob>,
    /// Cursor over raw rows, including markers rejected during decoding.
    pub next: Option<Uuid>,
}

pub(super) fn decode_sqlite_parked_job(
    row: SqliteRow,
) -> Result<Option<ParkedWorkspaceJob>, StorageError> {
    decode_parked_job_fields(
        row.try_get("id")?,
        row.try_get("workspace_id")?,
        row.try_get("payload_json")?,
        row.try_get("available_at")?,
        row.try_get("workspace_generation")?,
        row.try_get("state")?,
    )
}

pub(super) fn decode_postgres_parked_job(
    row: PgRow,
) -> Result<Option<ParkedWorkspaceJob>, StorageError> {
    decode_parked_job_fields(
        row.try_get("id")?,
        row.try_get("workspace_id")?,
        row.try_get("payload_json")?,
        row.try_get("available_at")?,
        row.try_get("workspace_generation")?,
        row.try_get("state")?,
    )
}

fn decode_parked_job_fields(
    id: String,
    workspace_id: String,
    payload_json: String,
    until: i64,
    workspace_generation: i64,
    state: String,
) -> Result<Option<ParkedWorkspaceJob>, StorageError> {
    let Ok(payload) = serde_json::from_str::<Value>(&payload_json) else {
        return Ok(None);
    };
    let Some(generation) = payload.get("reconciled_generation").and_then(Value::as_u64) else {
        return Ok(None);
    };
    let Some(reconciled_at) = payload.get("reconciled_at").and_then(Value::as_i64) else {
        return Ok(None);
    };
    let Some(wait) = payload.get("runtime_wait").and_then(Value::as_object) else {
        return Ok(None);
    };
    if u64::try_from(workspace_generation).ok() != Some(generation)
        || wait.get("until").and_then(Value::as_i64) != Some(until)
        || !wait
            .get("nonce")
            .and_then(Value::as_str)
            .is_some_and(|nonce| Uuid::parse_str(nonce).is_ok())
    {
        return Ok(None);
    }
    let desired_replicas = match state.as_str() {
        "provisioning" | "starting" | "restarting" => 1,
        "stopping" => 0,
        _ => return Ok(None),
    };
    Ok(Some(ParkedWorkspaceJob {
        id: Uuid::parse_str(&id)?,
        workspace_id: Uuid::parse_str(&workspace_id)?,
        payload_json,
        until,
        generation,
        reconciled_at,
        desired_replicas,
    }))
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
