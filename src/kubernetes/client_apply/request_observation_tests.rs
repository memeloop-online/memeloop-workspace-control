use std::{error::Error, fmt, io};

use super::{ErrorSummary, MAX_ERROR_CHAIN, ReconcileError};

#[test]
fn transparent_reconcile_wrapper_retains_transport_and_io_classification() {
    for (kind, expected) in [
        (io::ErrorKind::TimedOut, "timed_out"),
        (io::ErrorKind::ConnectionReset, "connection_reset"),
    ] {
        let error = ReconcileError::Kubernetes(kube::Error::Service(Box::new(io::Error::new(
            kind,
            "private injected value must never be formatted",
        ))));
        let summary = ErrorSummary::new(&error);
        assert_eq!(summary.kind, "service");
        assert_eq!(summary.io_kind, expected);
        assert!(summary.source_kinds.contains(&"io"));
        assert!(!format!("{summary:?}").contains("private injected"));
    }
}

#[test]
fn api_classification_exposes_only_status_code_even_with_private_status_text() {
    let error = kube::Error::Api(Box::new(
        serde_json::from_value(serde_json::json!({
            "apiVersion": "v1", "kind": "Status", "status": "Failure",
            "message": "private response payload", "reason": "private reason", "code": 503,
        }))
        .unwrap(),
    ));
    let summary = ErrorSummary::new(&error);
    assert_eq!(summary.kind, "api");
    assert_eq!(summary.api_code, Some(503));
    assert!(!format!("{summary:?}").contains("private"));
}

#[derive(Debug)]
struct Unprintable;
impl fmt::Display for Unprintable {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("classifiers must not format unknown errors");
    }
}
impl Error for Unprintable {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        // A pathological third-party error must not hang reconciliation logging.
        Some(self)
    }
}

#[test]
fn unknown_and_cyclic_sources_are_bounded_without_display_or_string_matching() {
    let summary = ErrorSummary::new(&Unprintable);
    assert_eq!(summary.kind, "unknown");
    assert_eq!(summary.io_kind, "none");
    assert_eq!(summary.source_kinds.len(), MAX_ERROR_CHAIN);
    assert!(summary.truncated);
}
