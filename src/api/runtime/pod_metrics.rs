use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use k8s_openapi::{api::core::v1::Pod, jiff::Timestamp};
use kube::{
    Api, Client,
    api::ListParams,
    core::{ApiResource, DynamicObject, GroupVersionKind},
};
use tokio::{sync::watch, time::MissedTickBehavior};

use crate::{
    config::InstallationId, kubernetes::OWNER_INSTALLATION_LABEL,
    workspace_runtime::WORKSPACE_NAMESPACE,
};

use super::{
    PodMetric, PodMetricMap, memory_metrics,
    pod_views::{is_active_pod, object_workspace_id},
};

const REFRESH_INTERVAL: Duration = Duration::from_secs(15);
const MAX_CACHE_AGE: Duration = Duration::from_secs(45);
const SAMPLE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone)]
pub(in crate::api) struct PodMetricsCache(watch::Sender<Option<Arc<PodMetricsSnapshot>>>);

pub(super) struct PodMetricsSnapshot {
    fetched_at: Instant,
    pub(super) metrics: PodMetricMap,
}

impl Default for PodMetricsCache {
    fn default() -> Self {
        Self(watch::channel(None).0)
    }
}

impl PodMetricsCache {
    pub(super) fn snapshot(&self) -> Option<Arc<PodMetricsSnapshot>> {
        self.0
            .borrow()
            .as_ref()
            .filter(|snapshot| snapshot.fetched_at.elapsed() <= MAX_CACHE_AGE)
            .cloned()
    }

    pub(in crate::api) async fn collect(
        self,
        client: Client,
        installation: InstallationId,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let resource = pod_metrics_resource();
        let api = Api::<DynamicObject>::namespaced_with(client, WORKSPACE_NAMESPACE, &resource);
        let params =
            ListParams::default().labels(&format!("{OWNER_INSTALLATION_LABEL}={installation}"));
        let mut tick = tokio::time::interval(REFRESH_INTERVAL);
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let refresh = async {
            loop {
                tick.tick().await;
                let list = match tokio::time::timeout(SAMPLE_TIMEOUT, api.list(&params)).await {
                    Ok(Ok(list)) => list,
                    Ok(Err(error)) => {
                        tracing::debug!(%error, "Workspace Pod metrics could not be refreshed");
                        continue;
                    }
                    Err(_) => {
                        tracing::debug!("Workspace Pod metrics refresh timed out");
                        continue;
                    }
                };
                let mut metrics = PodMetricMap::new();
                for pod in list.items {
                    if let Some(id) = object_workspace_id(&pod.metadata.labels) {
                        metrics.entry(id).or_default().extend(metrics_from_pod(pod));
                    }
                }
                self.0.send_replace(Some(Arc::new(PodMetricsSnapshot {
                    fetched_at: Instant::now(),
                    metrics,
                })));
            }
        };
        tokio::select! {
            _ = shutdown.wait_for(|stopped| *stopped) => {}
            _ = refresh => {}
        }
    }
}

// StatefulSet Pod names are reused; an earlier incarnation's sample is not current usage.
pub(super) fn observed_pod_metrics(
    metrics: &[PodMetric],
    pods: &[Pod],
    observed_now: i64,
) -> Vec<PodMetric> {
    let mut metrics = metrics
        .iter()
        .filter(|metric| {
            let Some(observed) = metric
                .observed_at
                .as_deref()
                .and_then(|value| value.parse::<Timestamp>().ok())
            else {
                return false;
            };
            observed.as_second() <= observed_now
                && observed_now.saturating_sub(observed.as_second())
                    <= memory_metrics::STALE_AFTER_SECONDS
                && pods.iter().any(|pod| {
                    is_active_pod(pod)
                        && pod.metadata.name.as_deref() == Some(metric.pod.as_str())
                        && pod
                            .metadata
                            .creation_timestamp
                            .as_ref()
                            .is_none_or(|created| observed >= created.0)
                })
        })
        .cloned()
        .collect::<Vec<_>>();
    memory_metrics::annotate(&mut metrics, pods, observed_now);
    metrics
}

fn pod_metrics_resource() -> ApiResource {
    ApiResource::from_gvk(&GroupVersionKind::gvk("metrics.k8s.io", "v1beta1", "Pod"))
}

fn metrics_from_pod(pod: DynamicObject) -> Vec<PodMetric> {
    let pod_name = pod.metadata.name.unwrap_or_default();
    let observed_at = pod
        .data
        .get("timestamp")
        .and_then(|value| value.as_str())
        .map(str::to_owned);
    pod.data
        .get("containers")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .map(|container| PodMetric {
            memory_limit: None,
            memory_used_percent: None,
            observed_at: observed_at.clone(),
            pod: pod_name.clone(),
            container: container
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_owned(),
            cpu: container
                .pointer("/usage/cpu")
                .and_then(|value| value.as_str())
                .map(str::to_owned),
            memory: container
                .pointer("/usage/memory")
                .and_then(|value| value.as_str())
                .map(str::to_owned),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn snapshots_cannot_attribute_expired_or_previous_pod_metrics_to_a_running_workspace() {
        let now = "2026-11-26T20:30:00Z".parse::<Timestamp>().unwrap();
        let pod = |name, phase, created| {
            serde_json::from_value::<Pod>(json!({
                "metadata":{"name":name,"creationTimestamp":created},
                "status":{"phase":phase},
            }))
            .unwrap()
        };
        let metric = |name, time| PodMetric {
            pod: name,
            container: "workspace".into(),
            cpu: Some("500m".into()),
            memory: Some("500Mi".into()),
            memory_limit: None,
            memory_used_percent: None,
            observed_at: Some(time),
        };
        let samples = observed_pod_metrics(
            &[
                metric("active".into(), "2026-11-26T20:29:55Z".into()),
                metric("active".into(), "2026-11-26T20:29:25Z".into()),
                metric("old".into(), "2026-11-26T20:27:59Z".into()),
                metric("completed".into(), "2026-11-26T20:29:55Z".into()),
                metric("missing".into(), "2026-11-26T20:29:55Z".into()),
            ],
            &[
                pod("active", "Running", "2026-11-26T20:29:35Z"),
                pod("old", "Running", "2026-11-26T20:20:01Z"),
                pod("completed", "Succeeded", "2026-11-26T20:29:35Z"),
            ],
            now.as_second(),
        );
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].pod, "active");
        assert_eq!(
            samples[0].observed_at.as_deref(),
            Some("2026-11-26T20:29:55Z")
        );
    }
}
