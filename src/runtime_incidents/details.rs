//! Typed, bounded facts from kubelet Events. Source messages never leave this parser.
use k8s_openapi::api::core::v1::Event;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{SchedulingBlocker, safe_reason, scheduling_blockers, startup_gate_pending};
use crate::storage::RuntimeEventCategory;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RuntimeEventDetails {
    pub event_uid: Option<Uuid>,
    pub pod_uid: Option<Uuid>,
    pub reason: Option<String>,
    pub container: Option<RuntimeContainer>,
    pub scheduling_blockers: Vec<SchedulingBlocker>,
    pub diagnostic: Option<RuntimeDiagnostic>,
}

impl RuntimeEventDetails {
    pub(crate) fn is_informative(&self, category: RuntimeEventCategory) -> bool {
        category != RuntimeEventCategory::Other
            || self.reason.is_some()
            || self.diagnostic.is_some()
            || !self.scheduling_blockers.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeContainer {
    Workspace,
    Ttyd,
    Buildkitd,
    WorkspaceScratchInit,
    WorkspaceBootstrap,
    BuildkitBootstrap,
}

impl RuntimeContainer {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "workspace" => Some(Self::Workspace),
            "ttyd" => Some(Self::Ttyd),
            "buildkitd" => Some(Self::Buildkitd),
            "workspace-scratch-init" => Some(Self::WorkspaceScratchInit),
            "workspace-bootstrap" => Some(Self::WorkspaceBootstrap),
            "buildkit-bootstrap" => Some(Self::BuildkitBootstrap),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeDiagnostic {
    ContainerLocalEphemeralLimit {
        container: RuntimeContainer,
        limit_bytes: u64,
    },
    PodLocalEphemeralLimit {
        limit_bytes: u64,
    },
    NodeResourcePressure {
        resource: RuntimePressureResource,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RuntimePressureResource {
    EphemeralStorage,
    Memory,
    Pids,
}

pub(crate) fn event_details(event: &Event) -> RuntimeEventDetails {
    let startup_pending = startup_gate_pending(event);
    let field = event.involved_object.field_path.as_deref();
    let container = field
        .and_then(|field| {
            field
                .strip_prefix("spec.containers{")
                .or_else(|| field.strip_prefix("spec.initContainers{"))
        })
        .and_then(|field| field.strip_suffix('}'))
        .and_then(RuntimeContainer::parse);
    RuntimeEventDetails {
        event_uid: event
            .metadata
            .uid
            .as_deref()
            .and_then(|uid| uid.parse().ok()),
        pod_uid: event
            .involved_object
            .uid
            .as_deref()
            .and_then(|uid| uid.parse().ok()),
        reason: if startup_pending {
            Some("ContainerCreating")
        } else {
            safe_reason(event.reason.as_deref())
        }
        .map(str::to_owned),
        container,
        scheduling_blockers: if event.reason.as_deref() == Some("FailedScheduling") {
            scheduling_blockers(event.message.as_deref())
        } else {
            Vec::new()
        },
        diagnostic: event.message.as_deref().and_then(diagnostic),
    }
}

// Kubernetes pkg/kubelet/eviction/helpers.go defines these exact message formats.
// Only parsed quantities and known enum values survive; never copy source text.
fn diagnostic(message: &str) -> Option<RuntimeDiagnostic> {
    let message = message.trim_end_matches(' ');
    if let Some(value) = message.strip_prefix("Container ") {
        let (container, limit) =
            value.split_once(" exceeded its local ephemeral storage limit \"")?;
        return Some(RuntimeDiagnostic::ContainerLocalEphemeralLimit {
            container: RuntimeContainer::parse(container)?,
            limit_bytes: bytes(limit.strip_suffix("\".")?)?,
        });
    }
    if let Some(limit) = message
        .strip_prefix("Pod ephemeral local storage usage exceeds the total limit of containers ")
    {
        return Some(RuntimeDiagnostic::PodLocalEphemeralLimit {
            limit_bytes: bytes(limit.strip_suffix('.')?)?,
        });
    }
    let value = message.strip_prefix("The node was low on resource: ")?;
    let (resource, _) = value.split_once('.')?;
    let resource = match resource {
        "ephemeral-storage" => RuntimePressureResource::EphemeralStorage,
        "memory" => RuntimePressureResource::Memory,
        "pids" => RuntimePressureResource::Pids,
        _ => return None,
    };
    // The anchored kubelet sentence establishes the resource. The suffix may
    // contain usage statistics or private text; none of it enters the result.
    Some(RuntimeDiagnostic::NodeResourcePressure { resource })
}

fn bytes(quantity: &str) -> Option<u64> {
    crate::kubernetes::home_volume_capacity_bytes(quantity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_kubelet_messages_produce_only_typed_facts() {
        assert_eq!(
            diagnostic("Container workspace exceeded its local ephemeral storage limit \"2Gi\". "),
            Some(RuntimeDiagnostic::ContainerLocalEphemeralLimit {
                container: RuntimeContainer::Workspace,
                limit_bytes: 2 * 1024 * 1024 * 1024
            })
        );
        assert_eq!(
            diagnostic(
                "Pod ephemeral local storage usage exceeds the total limit of containers 3Gi. "
            ),
            Some(RuntimeDiagnostic::PodLocalEphemeralLimit {
                limit_bytes: 3 * 1024 * 1024 * 1024
            })
        );
        for (name, resource) in [
            (
                "ephemeral-storage",
                RuntimePressureResource::EphemeralStorage,
            ),
            ("memory", RuntimePressureResource::Memory),
            ("pids", RuntimePressureResource::Pids),
        ] {
            assert_eq!(
                diagnostic(&format!(
                    "The node was low on resource: {name}. Threshold quantity: 100Mi, available: 50Mi. "
                )),
                Some(RuntimeDiagnostic::NodeResourcePressure { resource })
            );
        }
        assert_eq!(
            diagnostic(
                "The node was low on resource: ephemeral-storage. Threshold quantity: 100Mi, available: 50Mi. Container workspace was using 2Gi, request is 512Mi, has larger consumption of ephemeral-storage. "
            ),
            Some(RuntimeDiagnostic::NodeResourcePressure {
                resource: RuntimePressureResource::EphemeralStorage
            })
        );
    }

    #[test]
    fn unknown_or_sensitive_text_never_becomes_diagnostic_data() {
        for message in [
            "Container private-token exceeded its local ephemeral storage limit \"2Gi\". ",
            "Container workspace exceeded its local ephemeral storage limit \"-2Gi\". ",
            "Container workspace exceeded its local ephemeral storage limit \"2Gi\". https://secret.invalid/token",
            "Container workspace exceeded its local ephemeral storage limit \"1m\". ",
            "Container workspace exceeded its local ephemeral storage limit \"999999999999999999999Gi\". ",
            "The node was low on resource: private-resource. private probe output",
            "Readiness probe failed: https://secret.invalid/token",
        ] {
            assert_eq!(diagnostic(message), None);
        }
    }
}
