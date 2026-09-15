use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use reqwest::{Client, Url, redirect::Policy};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;

use crate::observability::{Observability, UpstreamKind};

const QUERY_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_SAMPLES: usize = 1_000;
const STALE_AFTER_SECONDS: i64 = 5 * 60;
const MAX_IDENTITIES: usize = 100;
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
///
/// `NodeLocal` is the bounded `emptyDir` compatibility mode. Kubernetes does
/// not expose reliable per-volume byte usage for that mode, so its telemetry
/// coverage is always unavailable rather than a synthetic zero.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StorageBacking {
    PersistentVolume,
    EphemeralVolume,
    NodeLocal,
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

pub(super) struct StorageMetricBatch {
    status: StorageTelemetryCoverage,
    used: MetricMap,
    capacity: MetricMap,
    available: MetricMap,
}

impl StorageMetricBatch {
    pub(super) fn telemetry(
        &self,
        identity: Option<&StorageIdentity>,
        configured_bytes: u64,
        backing: StorageBacking,
        now: i64,
    ) -> StorageTelemetry {
        if !matches!(self.status, StorageTelemetryCoverage::Exact) {
            return StorageTelemetry::unavailable(configured_bytes, self.status, backing);
        }
        let Some(key) = identity else {
            return StorageTelemetry::unavailable(
                configured_bytes,
                StorageTelemetryCoverage::Unavailable,
                backing,
            );
        };
        let (Some(used), Some(capacity), Some(available)) = (
            self.used.get(key),
            self.capacity.get(key),
            self.available.get(key),
        ) else {
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
        let used_percent = if capacity.value == 0 {
            None
        } else {
            Some((used.value as f64 / capacity.value as f64 * 100.0).clamp(0.0, 100.0))
        };
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

pub(super) type StorageIdentity = (String, String);
type MetricMap = BTreeMap<StorageIdentity, Sample>;

struct FetchedMetrics {
    used: MetricMap,
    capacity: MetricMap,
    available: MetricMap,
}

#[derive(Debug, Error)]
pub(super) enum StorageMetricError {
    #[error("Prometheus request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("Prometheus returned HTTP {0}")]
    Http(reqwest::StatusCode),
    #[error("Prometheus response exceeds the configured size limit")]
    ResponseTooLarge,
    #[error("Prometheus response is invalid")]
    InvalidResponse,
    #[error("too many PVC identities were requested")]
    TooManyIdentities,
}

pub(super) async fn fetch(
    base_url: Option<&Url>,
    identities: &[StorageIdentity],
    observability: &Observability,
) -> StorageMetricBatch {
    let Some(base_url) = base_url else {
        return empty_batch(StorageTelemetryCoverage::Disabled);
    };
    if identities.is_empty() {
        return empty_batch(StorageTelemetryCoverage::Unavailable);
    }
    match fetch_configured(base_url, identities, observability).await {
        Ok(metrics) => StorageMetricBatch {
            status: StorageTelemetryCoverage::Exact,
            used: metrics.used,
            capacity: metrics.capacity,
            available: metrics.available,
        },
        Err(error) => {
            tracing::debug!(%error, "Prometheus PVC telemetry is unavailable");
            empty_batch(StorageTelemetryCoverage::Unavailable)
        }
    }
}

fn empty_batch(status: StorageTelemetryCoverage) -> StorageMetricBatch {
    StorageMetricBatch {
        status,
        used: MetricMap::new(),
        capacity: MetricMap::new(),
        available: MetricMap::new(),
    }
}

async fn fetch_configured(
    base_url: &Url,
    identities: &[StorageIdentity],
    observability: &Observability,
) -> Result<FetchedMetrics, StorageMetricError> {
    let identities = identities.iter().cloned().collect::<BTreeSet<_>>();
    if identities.len() > MAX_IDENTITIES {
        return Err(StorageMetricError::TooManyIdentities);
    }
    let client = Client::builder()
        .timeout(QUERY_TIMEOUT)
        .redirect(Policy::none())
        .build()?;
    let used_query = metric_query("kubelet_volume_stats_used_bytes", &identities);
    let capacity_query = metric_query("kubelet_volume_stats_capacity_bytes", &identities);
    let available_query = metric_query("kubelet_volume_stats_available_bytes", &identities);
    let (used, capacity, available) = tokio::try_join!(
        query(&client, base_url, &used_query, &identities, observability),
        query(
            &client,
            base_url,
            &capacity_query,
            &identities,
            observability
        ),
        query(
            &client,
            base_url,
            &available_query,
            &identities,
            observability
        ),
    )?;
    Ok(FetchedMetrics {
        used,
        capacity,
        available,
    })
}

fn metric_query(metric: &str, identities: &BTreeSet<StorageIdentity>) -> String {
    identities
        .iter()
        .map(|(namespace, pvc)| {
            format!(
                "{metric}{{namespace=\"{}\",persistentvolumeclaim=\"{}\"}}",
                promql_label_value(namespace),
                promql_label_value(pvc),
            )
        })
        .collect::<Vec<_>>()
        .join(" or ")
}

fn promql_label_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            _ => escaped.push(character),
        }
    }
    escaped
}

async fn query(
    client: &Client,
    base_url: &Url,
    expression: &str,
    identities: &BTreeSet<StorageIdentity>,
    observability: &Observability,
) -> Result<MetricMap, StorageMetricError> {
    let request = observability.begin_upstream(UpstreamKind::Prometheus);
    let mut url = base_url.clone();
    let path = format!("{}/api/v1/query", url.path().trim_end_matches('/'));
    url.set_path(&path);
    url.query_pairs_mut().append_pair("query", expression);
    let mut response = client.get(url).send().await?;
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
    let parsed = parse_response(&body, identities)?;
    request.success();
    Ok(parsed)
}

#[derive(Deserialize)]
struct QueryResponse {
    status: String,
    data: QueryData,
}

#[derive(Deserialize)]
struct QueryData {
    #[serde(rename = "resultType")]
    result_type: String,
    result: Vec<QuerySample>,
}

#[derive(Deserialize)]
struct QuerySample {
    metric: QueryLabels,
    value: (f64, String),
}

#[derive(Deserialize)]
struct QueryLabels {
    namespace: String,
    persistentvolumeclaim: String,
}

fn parse_response(
    body: &[u8],
    identities: &BTreeSet<StorageIdentity>,
) -> Result<MetricMap, StorageMetricError> {
    let response: QueryResponse =
        serde_json::from_slice(body).map_err(|_| StorageMetricError::InvalidResponse)?;
    if response.status != "success"
        || response.data.result_type != "vector"
        || response.data.result.len() > MAX_SAMPLES
    {
        return Err(StorageMetricError::InvalidResponse);
    }
    let mut samples = BTreeMap::new();
    for item in response.data.result {
        let identity = (item.metric.namespace, item.metric.persistentvolumeclaim);
        if !identities.contains(&identity)
            || !item.value.0.is_finite()
            || item.value.0 < 0.0
            || item.value.0 > i64::MAX as f64
        {
            return Err(StorageMetricError::InvalidResponse);
        }
        let value = item
            .value
            .1
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && *value >= 0.0 && *value <= u64::MAX as f64)
            .ok_or(StorageMetricError::InvalidResponse)?;
        let sample = Sample {
            value: value.round() as u64,
            observed_at: item.value.0.floor() as i64,
        };
        samples
            .entry(identity)
            .and_modify(|existing: &mut Sample| {
                if sample.observed_at > existing.observed_at {
                    *existing = sample;
                }
            })
            .or_insert(sample);
    }
    Ok(samples)
}

#[cfg(test)]
mod tests;
