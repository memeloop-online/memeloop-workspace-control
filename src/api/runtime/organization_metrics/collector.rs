use reqwest::Url;
use uuid::Uuid;

use crate::observability::Observability;

use super::OrganizationMetrics;
use super::query::{ScalarSample, client, promql_label_value, query_scalar};

#[derive(Debug, Clone)]
struct MetricScope {
    matcher: String,
    restricted: bool,
    impossible: bool,
}

impl MetricScope {
    fn new(installation_id: &str, organization_id: Uuid, template_ids: Option<&[Uuid]>) -> Self {
        let mut labels = vec![
            format!(
                "label_workspace_memeloop_dev_owner_installation=\"{}\"",
                promql_label_value(installation_id),
            ),
            format!("label_workspace_memeloop_dev_organization_id=\"{organization_id}\""),
        ];
        let (restricted, impossible) = match template_ids {
            None => (false, false),
            Some([]) => (true, true),
            Some(template_ids) => {
                labels.push(format!(
                    "label_workspace_memeloop_dev_template_id=~\"{}\"",
                    template_ids
                        .iter()
                        .map(Uuid::to_string)
                        .collect::<Vec<_>>()
                        .join("|"),
                ));
                (true, false)
            }
        };
        Self {
            matcher: labels.join(","),
            restricted,
            impossible,
        }
    }

    fn labels(&self) -> String {
        format!(
            "max by(namespace,pod) (kube_pod_labels{{{}}})",
            self.matcher
        )
    }

    fn active_pods(&self) -> String {
        format!(
            "max by(namespace,pod) ((kube_pod_status_phase{{phase=~\"Pending|Running\"}} == 1) * on(namespace,pod) group_left() {})",
            self.labels()
        )
    }

    fn pvc_labels(&self) -> String {
        format!(
            "max by(namespace,persistentvolumeclaim) (kube_persistentvolumeclaim_labels{{{}}})",
            self.matcher
        )
    }
}

struct QueryExpressions {
    cpu: String,
    memory: String,
    active_pods: String,
    expected_containers: String,
    cpu_containers: String,
    memory_containers: String,
    disk: String,
    pvcs: String,
}

impl QueryExpressions {
    fn for_scope(scope: &MetricScope) -> Self {
        let active_pods = scope.active_pods();
        let container_info = "max by(namespace,pod,container) (kube_pod_container_info{container!=\"\",container!=\"POD\"})";
        let cpu_metric = format!(
            "(max by(namespace,pod,container) (rate(container_cpu_usage_seconds_total{{container!=\"\",container!=\"POD\"}}[5m])) and on(namespace,pod,container) {container_info})"
        );
        let memory_metric = format!(
            "(max by(namespace,pod,container) (container_memory_working_set_bytes{{container!=\"\",container!=\"POD\"}}) and on(namespace,pod,container) {container_info})"
        );
        let cpu_containers =
            format!("count({cpu_metric} * on(namespace,pod) group_left() {active_pods})");
        let memory_containers =
            format!("count({memory_metric} * on(namespace,pod) group_left() {active_pods})");
        let expected_containers =
            format!("count({container_info} * on(namespace,pod) group_left() {active_pods})");
        let cpu =
            format!("1000 * sum({cpu_metric} * on(namespace,pod) group_left() {active_pods})");
        let memory = format!(
            "sum({memory_metric} * on(namespace,pod) group_left() {active_pods}) / 1048576"
        );
        let active_count = format!("count({active_pods})");
        let pvc_labels = scope.pvc_labels();
        let pvc_usage = "max by(namespace,persistentvolumeclaim) (kubelet_volume_stats_used_bytes)";
        let disk = format!(
            "sum({pvc_usage} * on(namespace,persistentvolumeclaim) group_left() {pvc_labels})"
        );
        let pvcs = format!(
            "count({pvc_usage} * on(namespace,persistentvolumeclaim) group_left() {pvc_labels})"
        );
        Self {
            cpu,
            memory,
            active_pods: active_count,
            expected_containers,
            cpu_containers,
            memory_containers,
            disk,
            pvcs,
        }
    }
}

pub(super) async fn collect(
    base_url: Option<&Url>,
    installation_id: &str,
    organization_id: Uuid,
    template_ids: Option<&[Uuid]>,
    expected_active: u64,
    expected_pvcs: u64,
    observability: &Observability,
) -> OrganizationMetrics {
    let scope = MetricScope::new(installation_id, organization_id, template_ids);
    if scope.impossible {
        return OrganizationMetrics::default();
    }
    let Some(base_url) = base_url else {
        return unavailable(scope.restricted);
    };
    let Ok(client) = client() else {
        return unavailable(scope.restricted);
    };
    let queries = QueryExpressions::for_scope(&scope);
    let (
        cpu,
        memory,
        active_pods,
        expected_containers,
        cpu_containers,
        memory_containers,
        disk,
        pvcs,
    ) = tokio::join!(
        query_scalar(&client, base_url, &queries.cpu, observability),
        query_scalar(&client, base_url, &queries.memory, observability),
        query_scalar(&client, base_url, &queries.active_pods, observability),
        query_scalar(
            &client,
            base_url,
            &queries.expected_containers,
            observability
        ),
        query_scalar(&client, base_url, &queries.cpu_containers, observability),
        query_scalar(&client, base_url, &queries.memory_containers, observability),
        query_scalar(&client, base_url, &queries.disk, observability),
        query_scalar(&client, base_url, &queries.pvcs, observability),
    );

    assemble_metrics(
        scope.restricted,
        cpu,
        memory,
        active_pods,
        expected_containers,
        cpu_containers,
        memory_containers,
        disk,
        pvcs,
        expected_active,
        expected_pvcs,
    )
}

fn unavailable(restricted: bool) -> OrganizationMetrics {
    OrganizationMetrics {
        template_labels_complete: !restricted,
        ..OrganizationMetrics::default()
    }
}

fn sample_count(sample: &ScalarSample) -> Option<u64> {
    if !sample.value.is_finite()
        || sample.value < 0.0
        || sample.value.fract() != 0.0
        || sample.value > u64::MAX as f64
    {
        return None;
    }
    Some(sample.value as u64)
}

fn sample_value(sample: &ScalarSample) -> Option<u64> {
    if !sample.value.is_finite() || sample.value < 0.0 {
        return None;
    }
    let rounded = sample.value.round();
    if rounded > u64::MAX as f64 {
        return None;
    }
    Some(rounded as u64)
}

#[allow(clippy::too_many_arguments)]
fn assemble_metrics(
    restricted: bool,
    cpu: Option<ScalarSample>,
    memory: Option<ScalarSample>,
    active_pods: Option<ScalarSample>,
    expected_containers: Option<ScalarSample>,
    cpu_containers: Option<ScalarSample>,
    memory_containers: Option<ScalarSample>,
    disk: Option<ScalarSample>,
    pvcs: Option<ScalarSample>,
    expected_active: u64,
    expected_pvcs: u64,
) -> OrganizationMetrics {
    let active_complete = active_pods
        .as_ref()
        .and_then(sample_count)
        .is_some_and(|count| count == expected_active);
    let containers_complete = match (
        expected_containers.as_ref().and_then(sample_count),
        cpu_containers.as_ref().and_then(sample_count),
        memory_containers.as_ref().and_then(sample_count),
    ) {
        (Some(expected), Some(cpu), Some(memory)) => expected == cpu && expected == memory,
        _ => false,
    };
    let pvc_count_complete = pvcs
        .as_ref()
        .and_then(sample_count)
        .is_some_and(|count| count == expected_pvcs);
    let template_labels_complete = if restricted {
        active_complete && pvc_count_complete
    } else {
        true
    };
    let metrics_complete = active_complete && containers_complete;
    let observed_at = [cpu.as_ref(), memory.as_ref(), disk.as_ref()]
        .into_iter()
        .flatten()
        .map(|sample| sample.observed_at)
        .min();
    OrganizationMetrics {
        cpu_millis: metrics_complete
            .then(|| cpu.as_ref().and_then(sample_value))
            .flatten(),
        memory_mib: metrics_complete
            .then(|| memory.as_ref().and_then(sample_value))
            .flatten(),
        disk_bytes: pvc_count_complete
            .then(|| disk.as_ref().and_then(sample_value))
            .flatten(),
        observed_at,
        template_labels_complete,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_scope_is_uuid_only_and_empty_scope_is_not_unrestricted() {
        let organization_id = Uuid::nil();
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);
        let scope = MetricScope::new("install", organization_id, Some(&[first, second]));
        assert!(scope.restricted);
        assert!(!scope.impossible);
        assert!(scope.matcher.contains(&first.to_string()));
        assert!(scope.matcher.contains(&second.to_string()));
        let empty = MetricScope::new("install", organization_id, Some(&[]));
        assert!(empty.restricted);
        assert!(empty.impossible);
        assert!(!empty.matcher.contains("template_id=~"));
    }

    #[test]
    fn expressions_deduplicate_scrapes_before_summing() {
        let scope = MetricScope::new("install", Uuid::nil(), None);
        let queries = QueryExpressions::for_scope(&scope);
        assert!(queries.cpu.contains("max by(namespace,pod,container)"));
        assert!(queries.memory.contains("max by(namespace,pod,container)"));
        assert!(
            queries
                .disk
                .contains("max by(namespace,persistentvolumeclaim)")
        );
        assert!(queries.cpu.contains("container!=\"\""));
        assert!(queries.cpu.contains("container!=\"POD\""));
        assert!(queries.active_pods.contains("== 1"));
        assert!(queries.cpu.contains("and on(namespace,pod,container)"));
    }

    fn sample(value: f64) -> Option<ScalarSample> {
        Some(ScalarSample {
            value,
            observed_at: 100,
        })
    }

    #[test]
    fn partial_or_mismatched_coverage_is_null_not_zero() {
        let metrics = assemble_metrics(
            true,
            sample(1000.0),
            sample(2.0),
            sample(1.0),
            sample(2.0),
            sample(1.0),
            sample(2.0),
            sample(123.0),
            sample(1.0),
            2,
            1,
        );
        assert_eq!(metrics.cpu_millis, None);
        assert_eq!(metrics.memory_mib, None);
        assert_eq!(metrics.disk_bytes, Some(123));
        assert!(!metrics.template_labels_complete);
    }

    #[test]
    fn pvc_count_mismatch_nulls_disk_but_keeps_complete_pod_scope() {
        let metrics = assemble_metrics(
            false,
            sample(1000.0),
            sample(2.0),
            sample(1.0),
            sample(2.0),
            sample(2.0),
            sample(2.0),
            sample(123.0),
            sample(0.0),
            1,
            1,
        );
        assert_eq!(metrics.cpu_millis, Some(1000));
        assert_eq!(metrics.memory_mib, Some(2));
        assert_eq!(metrics.disk_bytes, None);
        assert!(metrics.template_labels_complete);
    }

    #[test]
    fn missing_sidecar_metric_cannot_complete_cpu_or_memory() {
        let metrics = assemble_metrics(
            false,
            sample(1000.0),
            sample(2.0),
            sample(1.0),
            sample(2.0),
            sample(1.0),
            sample(2.0),
            sample(123.0),
            sample(1.0),
            1,
            1,
        );
        assert_eq!(metrics.cpu_millis, None);
        assert_eq!(metrics.memory_mib, None);
    }
}
