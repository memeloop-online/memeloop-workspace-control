use super::*;

fn identity(name: &str, created_at: i64) -> StorageIdentity {
    StorageIdentity {
        key: (WORKSPACE_NAMESPACE.to_owned(), name.to_owned()),
        created_at: Some(created_at),
    }
}

fn sample_body() -> String {
    serde_json::json!({
        "status": "success",
        "data": {
            "resultType": "matrix",
            "result": [
                {
                    "metric": {
                        "__name__": METRICS[0], "namespace": WORKSPACE_NAMESPACE,
                        "persistentvolumeclaim": "pvc-a", "instance": "old"
                    },
                    "values": [[1_000.0, "50"], [1_005.0, "NaN"]]
                },
                {
                    "metric": {
                        "__name__": METRICS[0], "namespace": WORKSPACE_NAMESPACE,
                        "persistentvolumeclaim": "pvc-a", "instance": "new"
                    },
                    "values": [[1_015.0, "90"]]
                },
                {
                    "metric": {
                        "__name__": METRICS[1], "namespace": WORKSPACE_NAMESPACE,
                        "persistentvolumeclaim": "pvc-a"
                    },
                    "values": [[1_010.0, "100"]]
                },
                {
                    "metric": {
                        "__name__": METRICS[2], "namespace": WORKSPACE_NAMESPACE,
                        "persistentvolumeclaim": "pvc-a"
                    },
                    "values": [[1_012.0, "10"]]
                },
                {
                    "metric": {
                        "__name__": METRICS[0], "namespace": "outside-workspace-namespace",
                        "persistentvolumeclaim": "pvc-a"
                    },
                    "values": [[1_020.0, "100000"]]
                }
            ]
        }
    })
    .to_string()
}

#[test]
fn snapshots_report_actual_sample_time_and_drop_precreation_values() {
    let snapshot = StorageMetricBatch {
        enabled: true,
        volumes: parse_response(sample_body().as_bytes()).unwrap(),
    };
    assert_eq!(snapshot.volumes.len(), 1);
    let current = identity("pvc-a", 990);
    let reading = |identity: &StorageIdentity, now| {
        snapshot.telemetry(Some(identity), 200, StorageBacking::PersistentVolume, now)
    };
    let exact = reading(&current, 1_100);
    assert!(matches!(exact.coverage, StorageTelemetryCoverage::Exact));
    assert_eq!(exact.observed_at, Some(1_010));
    assert_eq!(exact.used_bytes, Some(90));
    assert!(matches!(exact.pressure, Some(StoragePressure::Critical)));
    let stale = reading(&current, 2_000);
    assert!(matches!(stale.coverage, StorageTelemetryCoverage::Stale));
    assert_eq!(stale.used_bytes, Some(90));
    for claim in [identity("pvc-unknown", 980), identity("pvc-a", 1_010)] {
        let unavailable = reading(&claim, 1_100);
        assert!(matches!(
            unavailable.coverage,
            StorageTelemetryCoverage::Unavailable
        ));
        assert_eq!(unavailable.configured_bytes, 200);
        assert_eq!(unavailable.used_bytes, None);
    }
}

#[test]
fn storage_pressure_uses_warning_and_critical_thresholds() {
    assert!(matches!(storage_pressure(79.9), StoragePressure::Normal));
    assert!(matches!(storage_pressure(80.0), StoragePressure::Warning));
    assert!(matches!(storage_pressure(90.0), StoragePressure::Critical));
}
