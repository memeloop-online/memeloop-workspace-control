use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::{Client, Url, redirect::Policy};
use serde::Deserialize;

use crate::observability::{Observability, UpstreamKind};

pub(super) const QUERY_TIMEOUT: Duration = Duration::from_secs(3);
pub(super) const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
pub(super) const MAX_RESULT_ROWS: usize = 1_024;
pub(super) const STALE_AFTER_SECONDS: i64 = 5 * 60;

#[derive(Debug, Clone, Copy)]
pub(super) struct ScalarSample {
    pub(super) value: f64,
    pub(super) observed_at: i64,
}

#[derive(Debug, Deserialize)]
struct QueryResponse {
    status: String,
    data: QueryData,
}

#[derive(Debug, Deserialize)]
struct QueryData {
    #[serde(rename = "resultType")]
    result_type: String,
    result: Vec<QuerySample>,
}

#[derive(Debug, Deserialize)]
struct QuerySample {
    value: Vec<serde_json::Value>,
}

pub(super) fn client() -> Result<Client, reqwest::Error> {
    Client::builder()
        .timeout(QUERY_TIMEOUT)
        .redirect(Policy::none())
        .build()
}

pub(super) async fn query_scalar(
    client: &Client,
    base_url: &Url,
    expression: &str,
    observability: &Observability,
) -> Option<ScalarSample> {
    query_scalar_at(
        client,
        base_url,
        expression,
        observability,
        unix_timestamp(),
    )
    .await
}

async fn query_scalar_at(
    client: &Client,
    base_url: &Url,
    expression: &str,
    observability: &Observability,
    now: i64,
) -> Option<ScalarSample> {
    let request = observability.begin_upstream(UpstreamKind::Prometheus);
    let mut url = base_url.clone();
    url.set_path(&format!(
        "{}/api/v1/query",
        url.path().trim_end_matches('/')
    ));
    url.query_pairs_mut().append_pair("query", expression);
    let mut response = client.get(url).send().await.ok()?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    let sample = parse_scalar_body(&body, now)?;
    request.success();
    Some(sample)
}

fn parse_scalar_body(body: &[u8], now: i64) -> Option<ScalarSample> {
    let response: QueryResponse = serde_json::from_slice(body).ok()?;
    if response.status != "success"
        || response.data.result_type != "vector"
        || response.data.result.len() != 1
        || response.data.result.len() > MAX_RESULT_ROWS
    {
        return None;
    }
    let values = &response.data.result.first()?.value;
    if values.len() != 2 {
        return None;
    }
    let observed_at = values[0]
        .as_f64()
        .filter(|value| value.is_finite() && *value >= 0.0 && *value <= i64::MAX as f64)
        .map(|value| value.floor() as i64)
        .filter(|timestamp| {
            *timestamp <= now.saturating_add(STALE_AFTER_SECONDS)
                && now.saturating_sub(*timestamp) <= STALE_AFTER_SECONDS
        })?;
    let value = values[1]
        .as_str()?
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0 && *value <= u64::MAX as f64)?;
    Some(ScalarSample { value, observed_at })
}

pub(super) fn promql_label_value(value: &str) -> String {
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

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_prometheus_label_values() {
        assert_eq!(promql_label_value("a\\\"\nb"), "a\\\\\\\"\\nb");
    }

    #[test]
    fn rejects_nan_negative_stale_and_partial_vectors() {
        let valid = |value: &str, timestamp: i64| {
            format!(
                r#"{{"status":"success","data":{{"resultType":"vector","result":[{{"metric":{{}},"value":[{timestamp},"{value}"]}}]}}}}"#
            )
        };
        assert!(parse_scalar_body(valid("NaN", 1_000).as_bytes(), 1_000).is_none());
        assert!(parse_scalar_body(valid("-1", 1_000).as_bytes(), 1_000).is_none());
        assert!(parse_scalar_body(valid("1", 1).as_bytes(), 1_000).is_none());
        let partial = br#"{"status":"success","data":{"resultType":"vector","result":[]}}"#;
        assert!(parse_scalar_body(partial, 1_000).is_none());
    }
}
