use std::{collections::BTreeMap, time::Duration};

use reqwest::{Client, Url, redirect::Policy};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;

use crate::config::InstallationId;

const QUERY_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_SAMPLES: usize = 1_000;
const STALE_AFTER_SECONDS: i64 = 5 * 60;
const WORKSPACE_PVC_NAME: &str = "workspace-data-workspace-0";

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StorageTelemetryStatus {
    Available,
    Stale,
    Unavailable,
    Disabled,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct StorageTelemetry {
    pub(super) status: StorageTelemetryStatus,
    pub(super) used_bytes: Option<u64>,
    pub(super) capacity_bytes: Option<u64>,
    pub(super) available_bytes: Option<u64>,
    pub(super) observed_at: Option<i64>,
}

impl StorageTelemetry {
    fn empty(status: StorageTelemetryStatus) -> Self {
        Self {
            status,
            used_bytes: None,
            capacity_bytes: None,
            available_bytes: None,
            observed_at: None,
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
    pub(super) fn telemetry(&self, namespace: &str, now: i64) -> StorageTelemetry {
        if !matches!(self.status, StorageTelemetryStatus::Available) {
            return StorageTelemetry::empty(self.status);
        }
        let (Some(used), Some(capacity), Some(available)) = (
            self.used.get(namespace),
            self.capacity.get(namespace),
            self.available.get(namespace),
        ) else {
            return StorageTelemetry::empty(StorageTelemetryStatus::Unavailable);
        };
        let observed_at = used
            .observed_at
            .min(capacity.observed_at)
            .min(available.observed_at);
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
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Sample {
    value: u64,
    observed_at: i64,
}

type MetricMap = BTreeMap<String, Sample>;

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
}

pub(super) async fn fetch(
    base_url: Option<&Url>,
    installation_id: &InstallationId,
) -> StorageMetricBatch {
    let Some(base_url) = base_url else {
        return empty_batch(StorageTelemetryStatus::Disabled);
    };
    match fetch_configured(base_url, installation_id).await {
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
    installation_id: &InstallationId,
) -> Result<FetchedMetrics, StorageMetricError> {
    let client = Client::builder()
        .timeout(QUERY_TIMEOUT)
        .redirect(Policy::none())
        .build()?;
    let selector = format!(
        "{{namespace=~\"ws-{}-.*\",persistentvolumeclaim=\"{WORKSPACE_PVC_NAME}\"}}",
        installation_id.as_str()
    );
    let used_query = format!("kubelet_volume_stats_used_bytes{selector}");
    let capacity_query = format!("kubelet_volume_stats_capacity_bytes{selector}");
    let available_query = format!("kubelet_volume_stats_available_bytes{selector}");
    let (used, capacity, available) = tokio::try_join!(
        query(&client, base_url, &used_query),
        query(&client, base_url, &capacity_query),
        query(&client, base_url, &available_query),
    )?;
    Ok(FetchedMetrics {
        used,
        capacity,
        available,
    })
}

async fn query(
    client: &Client,
    base_url: &Url,
    expression: &str,
) -> Result<BTreeMap<String, Sample>, StorageMetricError> {
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
    parse_response(&body)
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

fn parse_response(body: &[u8]) -> Result<BTreeMap<String, Sample>, StorageMetricError> {
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
        if item.metric.persistentvolumeclaim != WORKSPACE_PVC_NAME
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
            .entry(item.metric.namespace)
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
        let body = br#"{"status":"success","data":{"resultType":"vector","result":[{"metric":{"namespace":"ws-test-01abc","persistentvolumeclaim":"workspace-data-workspace-0"},"value":[1787980000,"1073741824"]}]}}"#;
        let samples = parse_response(body).unwrap();
        assert_eq!(samples["ws-test-01abc"].value, 1_073_741_824);
        let wrong = body
            .windows(WORKSPACE_PVC_NAME.len())
            .position(|window| window == WORKSPACE_PVC_NAME.as_bytes())
            .map(|index| {
                let mut wrong = body.to_vec();
                wrong.splice(
                    index..index + WORKSPACE_PVC_NAME.len(),
                    b"other-volume".iter().copied(),
                );
                wrong
            })
            .unwrap();
        assert!(matches!(
            parse_response(&wrong),
            Err(StorageMetricError::InvalidResponse)
        ));
    }

    #[test]
    fn telemetry_reports_available_stale_and_missing_states() {
        let sample = Sample {
            value: 100,
            observed_at: 1_000,
        };
        let values = StorageMetricBatch {
            status: StorageTelemetryStatus::Available,
            used: BTreeMap::from([("ws-test-01abc".to_owned(), sample)]),
            capacity: BTreeMap::from([("ws-test-01abc".to_owned(), sample)]),
            available: BTreeMap::from([("ws-test-01abc".to_owned(), sample)]),
        };
        assert!(matches!(
            values.telemetry("ws-test-01abc", 1_100).status,
            StorageTelemetryStatus::Available
        ));
        assert!(matches!(
            values.telemetry("ws-test-01abc", 2_000).status,
            StorageTelemetryStatus::Stale
        ));
        assert!(matches!(
            values.telemetry("ws-test-missing", 1_100).status,
            StorageTelemetryStatus::Unavailable
        ));
    }
}
