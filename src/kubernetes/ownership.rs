use std::collections::BTreeMap;

use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use thiserror::Error;

use super::{OWNER_INSTALLATION_LABEL, WORKSPACE_ID_LABEL};

pub(super) fn verify(
    metadata: &ObjectMeta,
    installation_id: &str,
    workspace_id: &str,
) -> Result<(), OwnershipError> {
    let labels = metadata
        .labels
        .as_ref()
        .ok_or(OwnershipError::MissingLabels)?;
    check_label(labels, OWNER_INSTALLATION_LABEL, installation_id)?;
    check_label(labels, WORKSPACE_ID_LABEL, workspace_id)
}

fn check_label(
    labels: &BTreeMap<String, String>,
    key: &'static str,
    expected: &str,
) -> Result<(), OwnershipError> {
    match labels.get(key) {
        Some(actual) if actual == expected => Ok(()),
        actual => Err(OwnershipError::LabelMismatch {
            key,
            expected: expected.to_owned(),
            actual: actual.cloned(),
        }),
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum OwnershipError {
    #[error("resource has no ownership labels")]
    MissingLabels,
    #[error("ownership label {key} mismatch: expected {expected}, actual {actual:?}")]
    LabelMismatch {
        key: &'static str,
        expected: String,
        actual: Option<String>,
    },
}
