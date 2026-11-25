use serde::{Deserialize, Serialize};
use std::time::Duration;

use sqlx::{
    Row,
    postgres::{PgListener, PgRow},
    sqlite::SqliteRow,
};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::events::NewEvent;

use super::{Database, StorageError, workspace_store::template_filter_sql};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct EventRecord {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub workspace_id: Option<Uuid>,
    pub kind: String,
    pub payload: serde_json::Value,
    pub created_at: i64,
}

pub struct EventNotifier {
    inner: EventNotifierInner,
}

enum EventNotifierInner {
    Poll(tokio::time::Interval),
    Postgres {
        listener: PgListener,
        installation_id: String,
    },
}

impl EventNotifier {
    pub async fn wait(&mut self) -> Result<(), StorageError> {
        match &mut self.inner {
            EventNotifierInner::Poll(interval) => {
                interval.tick().await;
                Ok(())
            }
            EventNotifierInner::Postgres {
                listener,
                installation_id,
            } => loop {
                let notification = listener.recv().await?;
                if notification.payload() == installation_id {
                    return Ok(());
                }
            },
        }
    }

    pub fn fall_back_to_polling(&mut self) {
        self.inner = EventNotifierInner::Poll(tokio::time::interval(Duration::from_secs(1)));
    }
}

impl Database {
    pub async fn event_notifier(&self) -> Result<EventNotifier, StorageError> {
        let inner = match self {
            Self::Sqlite { .. } => {
                EventNotifierInner::Poll(tokio::time::interval(Duration::from_secs(1)))
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let mut listener = PgListener::connect_with(pool).await?;
                listener.listen("mwc_events").await?;
                EventNotifierInner::Postgres {
                    listener,
                    installation_id: installation_id.to_string(),
                }
            }
        };
        Ok(EventNotifier { inner })
    }

    pub async fn append_event(
        &self,
        event: NewEvent,
        now: i64,
    ) -> Result<EventRecord, StorageError> {
        event.validate().map_err(|_| StorageError::InvalidEvent)?;
        let record = EventRecord {
            id: Uuid::now_v7(),
            organization_id: event.organization_id,
            workspace_id: event.workspace_id,
            kind: event.kind,
            payload: event.payload,
            created_at: now,
        };
        let payload_json = serde_json::to_string(&record.payload)?;
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO events (id, installation_id, organization_id, workspace_id, kind, payload_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)")
                    .bind(record.id.to_string()).bind(installation_id.as_str())
                    .bind(record.organization_id.to_string()).bind(record.workspace_id.map(|id| id.to_string()))
                    .bind(&record.kind).bind(payload_json).bind(now).execute(pool).await?;
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query("INSERT INTO events (id, installation_id, organization_id, workspace_id, kind, payload_json, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7)")
                    .bind(record.id.to_string()).bind(installation_id.as_str())
                    .bind(record.organization_id.to_string()).bind(record.workspace_id.map(|id| id.to_string()))
                    .bind(&record.kind).bind(payload_json).bind(now).execute(pool).await?;
                sqlx::query("SELECT pg_notify('mwc_events', $1)")
                    .bind(installation_id.as_str())
                    .execute(pool)
                    .await?;
            }
        }
        Ok(record)
    }

    pub async fn list_events(
        &self,
        organization_id: Uuid,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<EventRecord>, StorageError> {
        self.list_scoped_events(organization_id, after, limit, None)
            .await
    }

    pub async fn list_scoped_events(
        &self,
        organization_id: Uuid,
        after: Option<Uuid>,
        limit: u32,
        allowed_template_ids: Option<&[Uuid]>,
    ) -> Result<Vec<EventRecord>, StorageError> {
        let limit = i64::from(limit.clamp(1, 1_000));
        let after = after.map(|id| id.to_string()).unwrap_or_default();
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let sql = event_page_sql("?1", "?2", "?3", "?4", '?', allowed_template_ids);
                let mut query = sqlx::query(&sql)
                    .bind(installation_id.as_str())
                    .bind(organization_id.to_string())
                    .bind(&after)
                    .bind(limit);
                for id in allowed_template_ids.unwrap_or_default() {
                    query = query.bind(id.to_string());
                }
                query
                    .fetch_all(pool)
                    .await?
                    .into_iter()
                    .map(decode_sqlite)
                    .collect()
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let sql = event_page_sql("$1", "$2", "$3", "$4", '$', allowed_template_ids);
                let mut query = sqlx::query(&sql)
                    .bind(installation_id.as_str())
                    .bind(organization_id.to_string())
                    .bind(&after)
                    .bind(limit);
                for id in allowed_template_ids.unwrap_or_default() {
                    query = query.bind(id.to_string());
                }
                query
                    .fetch_all(pool)
                    .await?
                    .into_iter()
                    .map(decode_postgres)
                    .collect()
            }
        }
    }
}

fn event_page_sql(
    installation: &str,
    organization: &str,
    after: &str,
    limit: &str,
    placeholder: char,
    allowed_template_ids: Option<&[Uuid]>,
) -> String {
    let visible_workspace = allowed_template_ids.map_or_else(String::new, |ids| {
        let templates = template_filter_sql(Some(ids), placeholder, 5);
        format!(
            " AND workspace_id IN (SELECT id FROM workspaces WHERE installation_id = {installation} \
             AND organization_id = {organization}{templates})"
        )
    });
    format!(
        "SELECT id, organization_id, workspace_id, kind, payload_json, created_at FROM events \
         WHERE installation_id = {installation} AND organization_id = {organization} \
         AND id > {after}{visible_workspace} ORDER BY id LIMIT {limit}"
    )
}

fn decode_sqlite(row: SqliteRow) -> Result<EventRecord, StorageError> {
    decode(&row)
}

fn decode_postgres(row: PgRow) -> Result<EventRecord, StorageError> {
    decode(&row)
}

fn decode<R: Row>(row: &R) -> Result<EventRecord, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    let workspace_id: Option<String> = row.try_get("workspace_id")?;
    Ok(EventRecord {
        id: Uuid::parse_str(&row.try_get::<String, _>("id")?)?,
        organization_id: Uuid::parse_str(&row.try_get::<String, _>("organization_id")?)?,
        workspace_id: workspace_id.map(|id| Uuid::parse_str(&id)).transpose()?,
        kind: row.try_get("kind")?,
        payload: serde_json::from_str(&row.try_get::<String, _>("payload_json")?)?,
        created_at: row.try_get("created_at")?,
    })
}
