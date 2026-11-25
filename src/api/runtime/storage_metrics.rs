use std::{collections::BTreeMap, sync::Arc, time::Duration};

use k8s_openapi::api::core::v1::PersistentVolumeClaim;
use reqwest::{Client, Url, redirect::Policy};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{sync::watch, time::MissedTickBehavior};
use utoipa::ToSchema;

use crate::{
    observability::{Observability, UpstreamKind},
    workspace_runtime::WORKSPACE_NAMESPACE,
};

const REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const QUERY_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const STALE_AFTER_SECONDS: i64 = 5 * 60;
const METRICS: [&str; 3] = [
    "kubelet_volume_stats_used_bytes",
    "kubelet_volume_stats_capacity_bytes",
    "kubelet_volume_stats_available_bytes",
];
const WARNING_PERCENT: f64 = 80.0;
const CRITICAL_PERCENT: f64 = 90.0;

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StorageTelemetryCoverage {
    Exact,
    Stale,
    Unavailable,
    Disabled,
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoragePressure {
    Normal,
    Warning,
    Critical,
}

/// The storage implementation behind one product-level storage allocation.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StorageBacking {
    PersistentVolume,
    EphemeralVolume,
    Unknown,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct StorageTelemetry {
    pub(super) configured_bytes: u64,
    pub(super) used_bytes: Option<u64>,
    pub(super) capacity_bytes: Option<u64>,
    pub(super) available_bytes: Option<u64>,
    pub(super) observed_at: Option<i64>,
    pub(super) used_percent: Option<f64>,
    pub(super) pressure: Option<StoragePressure>,
    pub(super) coverage: StorageTelemetryCoverage,
    pub(super) backing: StorageBacking,
}

impl StorageTelemetry {
    fn unavailable(
        configured_bytes: u64,
        coverage: StorageTelemetryCoverage,
        backing: StorageBacking,
    ) -> Self {
        Self {
            configured_bytes,
            used_bytes: None,
            capacity_bytes: None,
            available_bytes: None,
            observed_at: None,
            used_percent: None,
            pressure: None,
            coverage,
            backing,
        }
    }
}

#[derive(Clone)]
pub(in crate::api) struct StorageMetricsCache(watch::Sender<Arc<StorageMetricBatch>>);

impl StorageMetricsCache {
    pub(in crate::api) fn new(enabled: bool) -> Self {
        Self(
            watch::channel(Arc::new(StorageMetricBatch {
                enabled,
                volumes: BTreeMap::new(),
            }))
            .0,
        )
    }

    pub(super) fn snapshot(&self) -> Arc<StorageMetricBatch> {
        self.0.borrow().clone()
    }

    pub(in crate::api) async fn collect(
        self,
        base_url: Url,
        observability: Observability,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let client = match Client::builder()
            .timeout(QUERY_TIMEOUT)
            .redirect(Policy::none())
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!(%error, "Could not initialize the Prometheus storage collector");
                return;
            }
        };
        let url = query_url(base_url);
        let mut tick = tokio::time::interval(REFRESH_INTERVAL);
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        while !*shutdown.borrow() {
            tokio::select! {
                _ = shutdown.changed() => break,
                _ = tick.tick() => {}
            }
            tokio::select! {
                _ = shutdown.changed() => break,
                result = fetch(&client, &url, &observability) => match result {
                    Ok(volumes) => {
                        self.0.send_replace(Arc::new(StorageMetricBatch { enabled: true, volumes }));
                    }
                    Err(error) => tracing::debug!(%error, "Prometheus PVC telemetry could not be refreshed"),
                }
            }
        }
    }
}

pub(super) struct StorageMetricBatch {
    enabled: bool,
    volumes: VolumeMetrics,
}

impl StorageMetricBatch {
    pub(super) fn telemetry(
        &self,
        identity: Option<&StorageIdentity>,
        configured_bytes: u64,
        backing: StorageBacking,
        now: i64,
    ) -> StorageTelemetry {
        if !self.enabled {
            return StorageTelemetry::unavailable(
                configured_bytes,
                StorageTelemetryCoverage::Disabled,
                backing,
            );
        }
        let Some((identity, &[Some(used), Some(capacity), Some(available)])) =
            identity.and_then(|identity| {
                self.volumes
                    .get(&identity.key)
                    .map(|samples| (identity, samples))
            })
        else {
            return StorageTelemetry::unavailable(
                configured_bytes,
                StorageTelemetryCoverage::Unavailable,
                backing,
            );
        };
        let observed_at = used
            .observed_at
            .min(capacity.observed_at)
            .min(available.observed_at);
        if identity
            .created_at
            .is_some_and(|created_at| observed_at <= created_at)
        {
            return StorageTelemetry::unavailable(
                configured_bytes,
                StorageTelemetryCoverage::Unavailable,
                backing,
            );
        }
        let used_percent = (capacity.value != 0)
            .then(|| (used.value as f64 / capacity.value as f64 * 100.0).clamp(0.0, 100.0));
        StorageTelemetry {
            configured_bytes,
            coverage: if now.saturating_sub(observed_at) > STALE_AFTER_SECONDS {
                StorageTelemetryCoverage::Stale
            } else {
                StorageTelemetryCoverage::Exact
            },
            used_bytes: Some(used.value),
            capacity_bytes: Some(capacity.value),
            available_bytes: Some(available.value),
            observed_at: Some(observed_at),
            used_percent,
            pressure: used_percent.map(storage_pressure),
            backing,
        }
    }
}

fn storage_pressure(used_percent: f64) -> StoragePressure {
    if used_percent >= CRITICAL_PERCENT {
        StoragePressure::Critical
    } else if used_percent >= WARNING_PERCENT {
        StoragePressure::Warning
    } else {
        StoragePressure::Normal
    }
}

#[derive(Debug, Clone, Copy)]
struct Sample {
    value: u64,
    observed_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StorageIdentity {
    key: (String, String),
    created_at: Option<i64>,
}

impl StorageIdentity {
    pub(super) fn from_claim(claim: &PersistentVolumeClaim) -> Option<Self> {
        Some(Self {
            key: (
                claim.metadata.namespace.clone()?,
                claim.metadata.name.clone()?,
            ),
            created_at: claim
                .metadata
                .creation_timestamp
                .as_ref()
                .map(|time| time.0.as_second()),
        })
    }
}

type VolumeMetrics = BTreeMap<(String, String), [Option<Sample>; 3]>;

#[derive(Debug, Error)]
enum StorageMetricError {
    #[error("Prometheus request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("Prometheus returned HTTP {0}")]
    Http(reqwest::StatusCode),
    #[error("Prometheus response exceeds the configured size limit")]
    ResponseTooLarge,
    #[error("Prometheus response is invalid")]
    InvalidResponse,
}

fn query_url(mut url: Url) -> Url {
    let path = format!("{}/api/v1/query", url.path().trim_end_matches('/'));
    url.set_path(&path);
    // Range vectors report actual sample timestamps, unlike instant vectors.
    let query = format!(
        r#"{{__name__=~"kubelet_volume_stats_(used|capacity|available)_bytes",namespace="{WORKSPACE_NAMESPACE}"}}[2m]"#
    );
    url.query_pairs_mut().append_pair("query", &query);
    url
}

async fn fetch(
    client: &Client,
    url: &Url,
    observability: &Observability,
) -> Result<VolumeMetrics, StorageMetricError> {
    let request = observability.begin_upstream(UpstreamKind::Prometheus);
    let mut response = client.get(url.clone()).send().await?;
    if !response.status().is_success() {
        return Err(StorageMetricError::Http(response.status()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(StorageMetricError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    let parsed = parse_response(&body)?;
    request.success();
    Ok(parsed)
}

#[derive(Deserialize)]
struct QueryResponse {
    status: String,
    data: QueryData,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueryData {
    result_type: String,
    result: Vec<QuerySamples>,
}

#[derive(Deserialize)]
struct QuerySamples {
    metric: QueryLabels,
    values: Vec<(f64, String)>,
}

#[derive(Deserialize)]
struct QueryLabels {
    #[serde(rename = "__name__")]
    name: String,
    namespace: String,
    persistentvolumeclaim: String,
}

fn parse_response(body: &[u8]) -> Result<VolumeMetrics, StorageMetricError> {
    let response: QueryResponse =
        serde_json::from_slice(body).map_err(|_| StorageMetricError::InvalidResponse)?;
    if response.status != "success" || response.data.result_type != "matrix" {
        return Err(StorageMetricError::InvalidResponse);
    }
    let mut volumes = VolumeMetrics::new();
    for item in response.data.result {
        let labels = item.metric;
        let Some(metric) = METRICS.iter().position(|name| *name == labels.name) else {
            continue;
        };
        if labels.namespace != WORKSPACE_NAMESPACE {
            continue;
        }
        let observation = &mut volumes
            .entry((labels.namespace, labels.persistentvolumeclaim))
            .or_default()[metric];
        for (timestamp, value) in item.values {
            let Some(value) = value
                .parse::<f64>()
                .ok()
                .filter(|value| (0.0..=u64::MAX as f64).contains(value))
            else {
                continue;
            };
            if (0.0..=i64::MAX as f64).contains(&timestamp) {
                let observed_at = timestamp.floor() as i64;
                if observation.is_none_or(|old| observed_at >= old.observed_at) {
                    *observation = Some(Sample {
                        value: value.round() as u64,
                        observed_at,
                    });
                }
            }
        }
    }
    Ok(volumes)
}

#[cfg(test)]
mod tests;
