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
pub(crate) enum StorageTelemetryStatus {
    Available,
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

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct StorageTelemetry {
    pub(super) status: StorageTelemetryStatus,
    pub(super) used_bytes: Option<u64>,
    pub(super) capacity_bytes: Option<u64>,
    pub(super) available_bytes: Option<u64>,
    pub(super) observed_at: Option<i64>,
    pub(super) used_percent: Option<f64>,
    pub(super) pressure: Option<StoragePressure>,
}

impl StorageTelemetry {
    fn empty(status: StorageTelemetryStatus) -> Self {
        Self {
            status,
            used_bytes: None,
            capacity_bytes: None,
            available_bytes: None,
            observed_at: None,
            used_percent: None,
            pressure: None,
        }
    }
}

pub(super) struct StorageMetricBatch {
    status: StorageTelemetryStatus,
    used: MetricMap,
    capacity: MetricMap,
    available: MetricMap,
}

impl StorageMetricBatch {
    pub(super) fn telemetry(&self, namespace: &str, pvc: &str, now: i64) -> StorageTelemetry {
        if !matches!(self.status, StorageTelemetryStatus::Available) {
            return StorageTelemetry::empty(self.status);
        }
        let key = (namespace.to_owned(), pvc.to_owned());
        let (Some(used), Some(capacity), Some(available)) = (
            self.used.get(&key),
            self.capacity.get(&key),
            self.available.get(&key),
        ) else {
            return StorageTelemetry::empty(StorageTelemetryStatus::Unavailable);
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
            status: if now.saturating_sub(observed_at) > STALE_AFTER_SECONDS {
                StorageTelemetryStatus::Stale
            } else {
                StorageTelemetryStatus::Available
            },
            used_bytes: Some(used.value),
            capacity_bytes: Some(capacity.value),
            available_bytes: Some(available.value),
            observed_at: Some(observed_at),
            used_percent,
            pressure: used_percent.map(storage_pressure),
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
        return empty_batch(StorageTelemetryStatus::Disabled);
    };
    match fetch_configured(base_url, identities, observability).await {
        Ok(metrics) => StorageMetricBatch {
            status: StorageTelemetryStatus::Available,
            used: metrics.used,
            capacity: metrics.capacity,
            available: metrics.available,
        },
        Err(error) => {
            tracing::debug!(%error, "Prometheus PVC telemetry is unavailable");
            empty_batch(StorageTelemetryStatus::Unavailable)
        }
    }
}

fn empty_batch(status: StorageTelemetryStatus) -> StorageMetricBatch {
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
mod tests {
    use super::*;

    #[test]
    fn parses_vector_and_rejects_wrong_pvc() {
        let pvc = "workspace-data-workspace-0";
        let identities = BTreeSet::from([("shared".to_owned(), pvc.to_owned())]);
        let body = br#"{"status":"success","data":{"resultType":"vector","result":[{"metric":{"namespace":"shared","persistentvolumeclaim":"workspace-data-workspace-0"},"value":[1787980000,"1073741824"]}]}}"#;
        let samples = parse_response(body, &identities).unwrap();
        assert_eq!(
            samples[&("shared".to_owned(), pvc.to_owned())].value,
            1_073_741_824
        );
        let wrong = body
            .windows(pvc.len())
            .position(|window| window == pvc.as_bytes())
            .map(|index| {
                let mut wrong = body.to_vec();
                wrong.splice(index..index + pvc.len(), b"other-volume".iter().copied());
                wrong
            })
            .unwrap();
        assert!(matches!(
            parse_response(&wrong, &identities),
            Err(StorageMetricError::InvalidResponse)
        ));
    }

    #[test]
    fn query_enumerates_exact_namespace_and_pvc_pairs() {
        let identities = BTreeSet::from([
            ("shared".to_owned(), "workspace-data-w-one-0".to_owned()),
            ("shared".to_owned(), "workspace-data-w-two-0".to_owned()),
        ]);
        assert_eq!(
            metric_query("used", &identities),
            "used{namespace=\"shared\",persistentvolumeclaim=\"workspace-data-w-one-0\"} or used{namespace=\"shared\",persistentvolumeclaim=\"workspace-data-w-two-0\"}"
        );
        assert_eq!(promql_label_value("a\\\"\nb"), "a\\\\\\\"\\nb");
    }

    #[test]
    fn telemetry_reports_available_stale_and_missing_states() {
        let sample = Sample {
            value: 100,
            observed_at: 1_000,
        };
        let values = StorageMetricBatch {
            status: StorageTelemetryStatus::Available,
            used: BTreeMap::from([(("shared".to_owned(), "pvc-a".to_owned()), sample)]),
            capacity: BTreeMap::from([(("shared".to_owned(), "pvc-a".to_owned()), sample)]),
            available: BTreeMap::from([(("shared".to_owned(), "pvc-a".to_owned()), sample)]),
        };
        assert!(matches!(
            values.telemetry("shared", "pvc-a", 1_100).status,
            StorageTelemetryStatus::Available
        ));
        assert!(matches!(
            values.telemetry("shared", "pvc-a", 2_000).status,
            StorageTelemetryStatus::Stale
        ));
        assert!(matches!(
            values.telemetry("shared", "pvc-b", 1_100).status,
            StorageTelemetryStatus::Unavailable
        ));
        assert!(matches!(
            values.telemetry("shared", "pvc-a", 1_100).pressure,
            Some(StoragePressure::Critical)
        ));
    }

    #[test]
    fn storage_pressure_uses_warning_and_critical_thresholds() {
        assert!(matches!(storage_pressure(79.9), StoragePressure::Normal));
        assert!(matches!(storage_pressure(80.0), StoragePressure::Warning));
        assert!(matches!(storage_pressure(90.0), StoragePressure::Critical));
    }
}
