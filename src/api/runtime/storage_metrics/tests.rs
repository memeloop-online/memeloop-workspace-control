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
