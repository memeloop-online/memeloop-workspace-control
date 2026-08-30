use std::{collections::VecDeque, convert::Infallible, sync::Arc, time::Duration};

use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
};
use futures::{Stream, stream};
use serde::Deserialize;
use utoipa::IntoParams;
use uuid::Uuid;

use crate::{
    auth::Permission,
    storage::{Database, EventNotifier, EventRecord},
};

use super::{ApiError, AppState, auth::principal};

#[derive(Debug, Deserialize, IntoParams)]
pub(super) struct EventQuery {
    pub organization_id: Uuid,
}

struct EventStreamState {
    stream_guard: crate::observability::StreamGuard,
    database: Database,
    organization_id: Uuid,
    cursor: Option<Uuid>,
    pending: VecDeque<EventRecord>,
    pending_bytes: u64,
    notifier: EventNotifier,
    initial_query: bool,
}

#[utoipa::path(
    get,
    path = "/api/v1/events",
    params(EventQuery, ("Last-Event-ID" = Option<Uuid>, Header)),
    responses(
        (status = 200, description = "Durable server-sent event stream", content_type = "text/event-stream"),
        (status = 401, body = super::ErrorEnvelope),
        (status = 403, body = super::ErrorEnvelope)
    )
)]
pub(super) async fn stream(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<EventQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.allows(Permission::ReadWorkspace, query.organization_id) {
        return Err(ApiError::Forbidden);
    }
    let cursor = headers
        .get("Last-Event-ID")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok());
    let notifier = state.database.event_notifier().await?;
    let event_stream = stream::unfold(
        EventStreamState {
            stream_guard: state.observability.begin_sse(),
            database: state.database.clone(),
            organization_id: query.organization_id,
            cursor,
            pending: VecDeque::new(),
            pending_bytes: 0,
            notifier,
            initial_query: true,
        },
        next_event,
    );
    Ok(Sse::new(event_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

async fn next_event(
    mut state: EventStreamState,
) -> Option<(Result<Event, Infallible>, EventStreamState)> {
    loop {
        if let Some(record) = state.pending.pop_front() {
            state.pending_bytes = state
                .pending_bytes
                .saturating_sub(serialized_event_bytes(&record));
            state.stream_guard.set_buffer_bytes(state.pending_bytes);
            state.cursor = Some(record.id);
            let event = match Event::default()
                .id(record.id.to_string())
                .event(record.kind.clone())
                .json_data(&record)
            {
                Ok(event) => event,
                Err(error) => Event::default()
                    .event("stream.error")
                    .data(format!("event serialization failed: {error}")),
            };
            return Some((Ok(event), state));
        }
        if state.initial_query {
            state.initial_query = false;
        } else if let Err(error) = state.notifier.wait().await {
            tracing::error!(error = %error, "PostgreSQL SSE notification listener failed; falling back to polling");
            state.notifier.fall_back_to_polling();
            let event = Event::default()
                .event("stream.error")
                .data("event notification temporarily failed; polling enabled");
            return Some((Ok(event), state));
        }
        match state
            .database
            .list_events(state.organization_id, state.cursor, 100)
            .await
        {
            Ok(records) => {
                let bytes = records.iter().map(serialized_event_bytes).sum::<u64>();
                state.pending.extend(records);
                state.pending_bytes = state.pending_bytes.saturating_add(bytes);
                state.stream_guard.set_buffer_bytes(state.pending_bytes);
            }
            Err(error) => {
                tracing::error!(error = %error, "SSE event query failed");
                let event = Event::default()
                    .event("stream.error")
                    .data("event query temporarily failed");
                return Some((Ok(event), state));
            }
        }
    }
}

fn serialized_event_bytes(record: &EventRecord) -> u64 {
    serde_json::to_vec(record)
        .map_or(0, |value| value.len())
        .try_into()
        .unwrap_or(u64::MAX)
}
