//! Report memory headroom even when Kubernetes readiness remains true.
//!
//! Metrics-server reports working set, not memory.current or PSI. Preserve that
//! distinction: absent/stale metrics stay unknown and low working set must not
//! be presented as proof that reclaim stalls are absent.
use k8s_openapi::{api::core::v1::Pod, jiff::Timestamp};

use super::{PodMetric, pod_views::is_active_pod};
use crate::kubernetes::home_volume_capacity_bytes;

const STALE_AFTER_SECONDS: i64 = 120;

pub(super) fn annotate(metrics: &mut [PodMetric], pods: &[Pod], now: i64) {
    for metric in metrics {
        let container = pods
            .iter()
            .filter(|pod| is_active_pod(pod))
            .find_map(|pod| {
                if pod.metadata.name.as_deref() != Some(metric.pod.as_str()) {
                    return None;
                }
                let spec = pod
                    .spec
                    .as_ref()?
                    .containers
                    .iter()
                    .find(|container| container.name == metric.container)?;
                let applied = pod
                    .status
                    .as_ref()
                    .and_then(|status| status.container_statuses.as_ref())
                    .and_then(|statuses| {
                        statuses
                            .iter()
                            .find(|status| status.name == metric.container)
                    })
                    .and_then(|status| status.resources.as_ref());
                Some(applied.or(spec.resources.as_ref()))
            });
        metric.memory_limit = container
            .flatten()
            .and_then(|resources| resources.limits.as_ref())
            .and_then(|limits| limits.get("memory"))
            .map(|quantity| quantity.0.clone());
        metric.memory_used_percent = used_percent(metric, now).filter(|_| {
            // A Pod name is reused after replacement; an older metrics sample
            // must not be attributed to the newly created Pod's resource limit.
            let observed = metric
                .observed_at
                .as_deref()
                .and_then(|value| value.parse::<Timestamp>().ok());
            let created = pods
                .iter()
                .find(|pod| pod.metadata.name.as_deref() == Some(metric.pod.as_str()))
                .and_then(|pod| pod.metadata.creation_timestamp.as_ref())
                .map(|value| value.0);
            !matches!((observed, created), (Some(observed), Some(created)) if observed < created)
        });
    }
}

fn used_percent(metric: &PodMetric, now: i64) -> Option<f64> {
    let observed = metric
        .observed_at
        .as_deref()?
        .parse::<Timestamp>()
        .ok()?
        .as_second();
    if observed > now || now.saturating_sub(observed) > STALE_AFTER_SECONDS {
        return None;
    }
    let used = home_volume_capacity_bytes(metric.memory.as_deref()?)?;
    let limit = home_volume_capacity_bytes(metric.memory_limit.as_deref()?)?;
    (limit > 0).then(|| used as f64 / limit as f64 * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn metric() -> PodMetric {
        PodMetric {
            pod: "w-current-0".into(),
            container: "workspace".into(),
            cpu: None,
            memory: Some("9900Mi".into()),
            memory_limit: None,
            memory_used_percent: None,
            observed_at: Some("2026-11-04T20:00:00Z".into()),
        }
    }

    #[test]
    fn ready_pod_at_its_limit_reports_applied_limit_instead_of_template_or_requested_resize() {
        let pod: Pod = serde_json::from_value(json!({
            "metadata":{"name":"w-current-0"},
            "spec":{"containers":[{"name":"workspace","resources":{"limits":{"memory":"12Gi"}}}]},
            "status":{"phase":"Running","containerStatuses":[{"name":"workspace","ready":true,"restartCount":0,"image":"test","imageID":"test","resources":{"limits":{"memory":"10Gi"}}}]}
        })).unwrap();
        let mut metrics = vec![metric()];
        let now = "2026-11-04T20:01:00Z"
            .parse::<Timestamp>()
            .unwrap()
            .as_second();
        annotate(&mut metrics, &[pod], now);
        assert_eq!(metrics[0].memory_limit.as_deref(), Some("10Gi"));
        assert!(metrics[0].memory_used_percent.unwrap() > 96.0);
    }

    #[test]
    fn stale_missing_invalid_and_unlimited_usage_remain_unknown() {
        let now = "2026-11-04T20:01:00Z"
            .parse::<Timestamp>()
            .unwrap()
            .as_second();
        let mut value = metric();
        value.memory_limit = Some("10Gi".into());
        assert!(used_percent(&value, now).is_some());
        assert_eq!(used_percent(&value, now + 121), None);
        assert_eq!(used_percent(&value, now - 61), None);
        value.memory_limit = Some("0".into());
        assert_eq!(used_percent(&value, now), None);
        value.memory_limit = None;
        assert_eq!(used_percent(&value, now), None);
        value.memory_limit = Some("10Gi".into());
        value.memory = Some("invalid".into());
        assert_eq!(used_percent(&value, now), None);
        value.memory = Some("1Mi".into());
        value.observed_at = None;
        assert_eq!(used_percent(&value, now), None);
    }

    #[test]
    fn missing_applied_resources_uses_spec_limit_but_replaced_pod_rejects_old_sample() {
        let mut pod: Pod = serde_json::from_value(json!({
            "metadata":{"name":"w-current-0","creationTimestamp":"2026-11-04T19:00:00Z"},
            "spec":{"containers":[{"name":"workspace","resources":{"limits":{"memory":"10Gi"}}}]}
        }))
        .unwrap();
        let now = "2026-11-04T20:01:00Z"
            .parse::<Timestamp>()
            .unwrap()
            .as_second();
        let mut metrics = vec![metric()];
        annotate(&mut metrics, &[pod.clone()], now);
        assert_eq!(metrics[0].memory_limit.as_deref(), Some("10Gi"));
        assert!(metrics[0].memory_used_percent.is_some());
        pod.metadata.creation_timestamp =
            Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(
                "2026-11-04T20:00:30Z".parse::<Timestamp>().unwrap(),
            ));
        annotate(&mut metrics, &[pod], now);
        assert!(metrics[0].memory_used_percent.is_none());
        assert_eq!(metrics[0].memory.as_deref(), Some("9900Mi"));
    }

    #[test]
    fn missing_or_terminating_pod_cannot_lend_a_limit_to_a_metric() {
        let pod: Pod = serde_json::from_value(json!({
            "metadata":{"name":"w-current-0","deletionTimestamp":"2026-11-04T20:00:00Z"},
            "spec":{"containers":[{"name":"workspace","resources":{"limits":{"memory":"10Gi"}}}]}
        }))
        .unwrap();
        let mut metrics = vec![metric()];
        annotate(&mut metrics, &[pod], 0);
        assert!(metrics[0].memory_limit.is_none());
        assert!(metrics[0].memory_used_percent.is_none());
    }
}
