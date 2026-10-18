//! Recognize scheduler causes without returning messages, host names or volume identities.
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SchedulingBlocker {
    CpuCapacity,
    MemoryCapacity,
    StorageCapacity,
    NodeAffinity,
    VolumeAffinity,
    Taint,
    NodeUnavailable,
}

pub(crate) fn scheduling_blockers(message: Option<&str>) -> Vec<SchedulingBlocker> {
    let message = message.unwrap_or_default().to_ascii_lowercase();
    let matches = |phrases: &[&str]| phrases.iter().any(|phrase| message.contains(phrase));
    let mut blockers = Vec::new();
    for (blocker, present) in [
        (
            SchedulingBlocker::CpuCapacity,
            matches(&["insufficient cpu"]),
        ),
        (
            SchedulingBlocker::MemoryCapacity,
            matches(&["insufficient memory"]),
        ),
        (
            SchedulingBlocker::StorageCapacity,
            matches(&[
                "did not have enough free storage",
                "not enough free storage",
                "insufficient storage",
            ]),
        ),
        (
            SchedulingBlocker::NodeAffinity,
            matches(&[
                "didn't match pod's node affinity",
                "didn't match node selector",
            ]),
        ),
        (
            SchedulingBlocker::VolumeAffinity,
            matches(&[
                "volume node affinity conflict",
                "didn't match persistent volume's node affinity",
            ]),
        ),
        (
            SchedulingBlocker::Taint,
            matches(&["untolerated taint", "had taint"]),
        ),
        (
            SchedulingBlocker::NodeUnavailable,
            matches(&[
                "node(s) were unschedulable",
                "node(s) were not ready",
                "node(s) not ready",
            ]),
        ),
    ] {
        if present {
            blockers.push(blocker);
        }
    }
    blockers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_causes_are_specific_deduplicated_and_do_not_expose_identifiers() {
        let blockers = scheduling_blockers(Some(
            "0/8 nodes are available: 1 Insufficient cpu, 2 Insufficient cpu, 1 Insufficient memory, 3 node(s) didn't match Pod's node affinity/selector, 1 node(s) had volume node affinity conflict. private-host private-pvc",
        ));
        assert_eq!(
            blockers,
            vec![
                SchedulingBlocker::CpuCapacity,
                SchedulingBlocker::MemoryCapacity,
                SchedulingBlocker::NodeAffinity,
                SchedulingBlocker::VolumeAffinity
            ]
        );
        let json = serde_json::to_string(&blockers).unwrap();
        assert_eq!(
            json,
            r#"["cpu_capacity","memory_capacity","node_affinity","volume_affinity"]"#
        );
        assert!(!json.contains("private"));
    }

    #[test]
    fn csi_storage_capacity_is_separate_from_volume_topology() {
        assert_eq!(
            scheduling_blockers(Some(
                "0/8 nodes are available: 1 node(s) did not have enough free storage, 7 node(s) didn't match Pod's node affinity/selector."
            )),
            vec![
                SchedulingBlocker::StorageCapacity,
                SchedulingBlocker::NodeAffinity
            ]
        );
        assert_eq!(
            scheduling_blockers(Some("1 node(s) had volume node affinity conflict")),
            vec![SchedulingBlocker::VolumeAffinity]
        );
    }

    #[test]
    fn scheduler_constraints_override_generic_volume_and_memory_categories() {
        use crate::storage::RuntimeEventCategory;

        for message in [
            "1 Insufficient cpu",
            "1 Insufficient memory",
            "1 node(s) did not have enough free storage for volume private-pvc",
            "1 node(s) had volume node affinity conflict",
        ] {
            assert_eq!(
                super::super::runtime_event_category(Some("FailedScheduling"), Some(message)),
                RuntimeEventCategory::Scheduling,
            );
        }
        assert_eq!(
            super::super::runtime_event_category(
                Some("FailedScheduling"),
                Some(
                    "waiting for ephemeral volume controller to create persistentvolumeclaim private-pvc"
                ),
            ),
            RuntimeEventCategory::TemporaryStorageProvisioning,
        );
    }

    #[test]
    fn known_node_constraints_and_unknown_scheduler_messages_are_safe() {
        assert_eq!(
            scheduling_blockers(Some(
                "1 node(s) had untolerated taint {private.example/key: value}, 1 node(s) were unschedulable"
            )),
            vec![SchedulingBlocker::Taint, SchedulingBlocker::NodeUnavailable]
        );
        for message in [
            None,
            Some(""),
            Some("sensitive unknown scheduler diagnostic"),
            Some(
                "waiting for ephemeral volume controller to create persistentvolumeclaim private-pvc",
            ),
        ] {
            assert!(scheduling_blockers(message).is_empty());
        }
    }
}
