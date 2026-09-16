//! Collect normalized runtime history independently of tenant page visits.
//!
//! Events outlive their Pods, so map the immutable Pod name back to the
//! installation-scoped workspace instead of requiring the evicted Pod to exist.
use std::{collections::BTreeMap, time::Duration};

use k8s_openapi::api::core::v1::Event;
use kube::{Api, api::ListParams};
use tokio::sync::watch;

use crate::{
    storage::{Database, NewWorkspaceRuntimeIncident, RuntimeEventCategory, StorageError},
    workspace_runtime::WORKSPACE_NAMESPACE,
};

/// One namespace event scan per interval, paginated to bound response memory.
/// No per-workspace Kubernetes requests and no raw event text in logs/storage.
pub async fn collect_until_shutdown(
    client: kube::Client,
    database: Database,
    mut shutdown: watch::Receiver<bool>,
) {
    let api = Api::<Event>::namespaced(client, WORKSPACE_NAMESPACE);
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        if *shutdown.borrow() {
            return;
        }
        tokio::select! {
            result = shutdown.changed() => {
                if result.is_err() || *shutdown.borrow() {
                    return;
                }
            }
            _ = interval.tick() => {
                // Shutdown also cancels an in-flight Kubernetes/database request.
                tokio::select! {
                    _ = shutdown.changed() => return,
                    result = collect_once(&api, &database) => {
                        if let Err(stage) = result {
                            // Kubernetes error bodies can contain diagnostic data.
                            tracing::warn!(stage, "runtime incident collection failed; retrying next interval");
                        }
                    }
                }
            }
        }
    }
}

async fn collect_once(api: &Api<Event>, database: &Database) -> Result<(), &'static str> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "clock")?
        .as_secs() as i64;
    let mut cursor = String::new();
    loop {
        let mut params = ListParams::default()
            .fields("involvedObject.kind=Pod,type=Warning")
            .limit(500);
        if !cursor.is_empty() {
            params = params.continue_token(&cursor);
        }
        let page = api.list(&params).await.map_err(|_| "kubernetes")?;
        let mut grouped = BTreeMap::<String, Vec<NewWorkspaceRuntimeIncident>>::new();
        for event in page.items {
            let Some(short_id) = workspace_short_id(&event) else {
                continue;
            };
            let Some(incident) = normalized_incident(&event) else {
                continue;
            };
            grouped
                .entry(short_id.to_owned())
                .or_default()
                .push(incident);
        }
        for (short_id, incidents) in grouped {
            let route = format!("{}-{short_id}", database.installation_id());
            let workspace = match database.get_workspace_by_route_key(&route).await {
                Ok(workspace) => workspace,
                Err(StorageError::WorkspaceNotFound) => continue,
                Err(_) => return Err("workspace_lookup"),
            };
            database
                .upsert_workspace_runtime_incidents(workspace.id, &incidents, now)
                .await
                .map_err(|_| "persistence")?;
        }
        cursor = page.metadata.continue_.unwrap_or_default();
        if cursor.is_empty() {
            return Ok(());
        }
    }
}

fn workspace_short_id(event: &Event) -> Option<&str> {
    let object = &event.involved_object;
    if object.kind.as_deref() != Some("Pod")
        || object.namespace.as_deref() != Some(WORKSPACE_NAMESPACE)
    {
        return None;
    }
    let short_id = object
        .name
        .as_deref()?
        .strip_prefix("w-")?
        .strip_suffix("-0")?;
    (short_id.len() == 16
        && short_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then_some(short_id)
}

fn normalized_incident(event: &Event) -> Option<NewWorkspaceRuntimeIncident> {
    if event.type_.as_deref() != Some("Warning") {
        return None;
    }
    let category = runtime_event_category(event.reason.as_deref(), event.message.as_deref());
    if category == RuntimeEventCategory::Other {
        return None;
    }
    let observed_at = event_start(event)?;
    let last_observed_at = event
        .series
        .as_ref()
        .and_then(|series| series.last_observed_time.as_ref())
        .map(|time| time.0.as_second())
        .or_else(|| event.last_timestamp.as_ref().map(|time| time.0.as_second()))
        .unwrap_or(observed_at);
    let count = event
        .series
        .as_ref()
        .and_then(|series| series.count)
        .or(event.count)
        .and_then(|count| u32::try_from(count).ok())
        .unwrap_or(1)
        .max(1);
    Some(NewWorkspaceRuntimeIncident {
        category,
        observed_at,
        last_observed_at,
        count,
    })
}

/// A series' latest timestamp changes on every recurrence and must not be
/// part of the persisted incident key, including observations from API reads.
pub(crate) fn event_start(event: &Event) -> Option<i64> {
    event
        .event_time
        .as_ref()
        .map(|time| time.0.as_second())
        .or_else(|| {
            event
                .first_timestamp
                .as_ref()
                .map(|time| time.0.as_second())
        })
        .or_else(|| {
            event
                .metadata
                .creation_timestamp
                .as_ref()
                .map(|time| time.0.as_second())
        })
}

/// Classify in memory; never return the sensitive source message to callers.
pub(crate) fn runtime_event_category(
    reason: Option<&str>,
    message: Option<&str>,
) -> RuntimeEventCategory {
    let reason = reason.unwrap_or_default().to_ascii_lowercase();
    let message = message.unwrap_or_default().to_ascii_lowercase();
    let contains = |needle: &str| reason.contains(needle) || message.contains(needle);
    if contains("ephemeral-storage") || contains("ephemeral storage") {
        RuntimeEventCategory::EphemeralStorage
    } else if contains("memorypressure")
        || contains("memory pressure")
        || contains("low on resource: memory")
    {
        RuntimeEventCategory::MemoryPressure
    } else if contains("diskpressure") || contains("disk pressure") {
        RuntimeEventCategory::DiskPressure
    } else if contains("pidpressure")
        || contains("pid pressure")
        || contains("low on resource: pid")
    {
        RuntimeEventCategory::PidPressure
    } else if contains("evicted") {
        RuntimeEventCategory::Evicted
    } else if contains("provision")
        && (contains("workspace-scratch")
            || contains("ephemeral volume")
            || contains("persistentvolumeclaim"))
    {
        RuntimeEventCategory::TemporaryStorageProvisioning
    } else if contains("attach")
        && (contains("workspace-scratch")
            || contains("ephemeral volume")
            || contains("persistentvolumeclaim"))
    {
        RuntimeEventCategory::TemporaryStorageAttachment
    } else if contains("failedmount") || contains("failed mount") || contains("volume") {
        RuntimeEventCategory::VolumeUnavailable
    } else {
        RuntimeEventCategory::Other
    }
}

#[cfg(test)]
mod tests;
