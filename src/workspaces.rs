use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{templates::WorkspaceTemplateSpec, workspace_runtime::WorkspaceRuntimeIdentity};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceState {
    Provisioning,
    Ready,
    Stopping,
    Stopped,
    Starting,
    Restarting,
    Deleting,
    Deleted,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceAction {
    Start,
    Stop,
    Restart,
    Delete,
}

/// A state reported by the workspace reconciler after observing Kubernetes.
///
/// This is deliberately separate from [`WorkspaceAction`]: actions express user
/// intent and enqueue reconciliation, while observations only record its result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceObservation {
    Ready,
    Stopped,
    Deleted,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    Internal,
    Public,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Workspace {
    pub id: Uuid,
    pub short_id: String,
    pub organization_id: Uuid,
    pub owner_id: Uuid,
    pub name: String,
    pub template_id: Option<Uuid>,
    pub node_pool: String,
    pub runtime: WorkspaceRuntimeIdentity,
    #[serde(default)]
    pub home_volume_binding: Option<WorkspaceHomeVolumeBinding>,
    #[serde(flatten)]
    pub template: WorkspaceTemplateSpec,
    pub state: WorkspaceState,
    pub generation: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceHomeVolumeBinding {
    pub namespace: String,
    pub claim_name: String,
    pub claim_uid: String,
    pub capacity_gib: u64,
}

impl WorkspaceHomeVolumeBinding {
    pub fn is_valid(&self) -> bool {
        self.namespace == WorkspaceRuntimeIdentity.namespace()
            && !self.claim_name.is_empty()
            && self.claim_name.len() <= 253
            && self.claim_name.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && label.starts_with(|character: char| character.is_ascii_alphanumeric())
                    && label.ends_with(|character: char| character.is_ascii_alphanumeric())
                    && label.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                    })
            })
            && !self.claim_uid.is_empty()
            && self.claim_uid.len() <= 128
            && self
                .claim_uid
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            && self.capacity_gib > 0
            && self
                .capacity_gib
                .checked_mul(1 << 30)
                .is_some_and(|bytes| bytes <= i64::MAX as u64)
    }
}

/// Private Kubernetes scheduling material stored by a system-managed node pool.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ResolvedPlacement {
    pub selector: BTreeMap<String, String>,
    pub required_hosts: Vec<String>,
    pub preferred_hosts: Vec<String>,
}

impl WorkspaceState {
    pub fn request(self, action: WorkspaceAction) -> Result<Self, TransitionError> {
        use WorkspaceAction as Action;
        use WorkspaceState as State;

        let next = match (self, action) {
            (State::Stopped | State::Failed, Action::Start) => State::Starting,
            (
                State::Provisioning
                | State::Ready
                | State::Starting
                | State::Restarting
                | State::Failed,
                Action::Stop,
            ) => State::Stopping,
            (
                State::Provisioning | State::Ready | State::Starting | State::Failed,
                Action::Restart,
            ) => State::Restarting,
            (
                State::Provisioning
                | State::Ready
                | State::Stopped
                | State::Starting
                | State::Stopping
                | State::Restarting
                | State::Failed,
                Action::Delete,
            ) => State::Deleting,
            _ => {
                return Err(TransitionError {
                    state: self,
                    operation: action.as_str(),
                });
            }
        };
        Ok(next)
    }

    pub fn observe(self, observation: WorkspaceObservation) -> Result<Self, TransitionError> {
        use WorkspaceObservation as Observation;
        use WorkspaceState as State;

        let next = match (self, observation) {
            (State::Provisioning | State::Starting | State::Restarting, Observation::Ready) => {
                State::Ready
            }
            (State::Stopping, Observation::Stopped) => State::Stopped,
            (State::Deleting, Observation::Deleted) => State::Deleted,
            (
                State::Provisioning
                | State::Ready
                | State::Starting
                | State::Stopping
                | State::Restarting
                | State::Deleting,
                Observation::Failed,
            ) => State::Failed,
            _ => {
                return Err(TransitionError {
                    state: self,
                    operation: observation.as_str(),
                });
            }
        };
        Ok(next)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Provisioning => "provisioning",
            Self::Ready => "ready",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Restarting => "restarting",
            Self::Deleting => "deleting",
            Self::Deleted => "deleted",
            Self::Failed => "failed",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "provisioning" => Some(Self::Provisioning),
            "ready" => Some(Self::Ready),
            "stopping" => Some(Self::Stopping),
            "stopped" => Some(Self::Stopped),
            "starting" => Some(Self::Starting),
            "restarting" => Some(Self::Restarting),
            "deleting" => Some(Self::Deleting),
            "deleted" => Some(Self::Deleted),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

impl WorkspaceAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Delete => "delete",
        }
    }

    pub fn from_api(value: &str) -> Option<Self> {
        match value {
            "start" => Some(Self::Start),
            "stop" => Some(Self::Stop),
            "restart" => Some(Self::Restart),
            "delete" => Some(Self::Delete),
            _ => None,
        }
    }
}

impl WorkspaceObservation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "mark_ready",
            Self::Stopped => "mark_stopped",
            Self::Deleted => "mark_deleted",
            Self::Failed => "mark_failed",
        }
    }
}

impl AccessMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Internal => "internal",
            Self::Public => "public",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        match value {
            "internal" => Some(Self::Internal),
            "public" => Some(Self::Public),
            _ => None,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("workspace operation {operation} is invalid while state is {state:?}")]
pub struct TransitionError {
    pub state: WorkspaceState,
    pub operation: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_stop_and_restart_lifecycle() {
        let stopped = WorkspaceState::Ready
            .request(WorkspaceAction::Stop)
            .unwrap()
            .observe(WorkspaceObservation::Stopped)
            .unwrap();
        assert_eq!(stopped, WorkspaceState::Stopped);
        assert_eq!(
            stopped.request(WorkspaceAction::Start).unwrap(),
            WorkspaceState::Starting
        );
        assert_eq!(
            WorkspaceState::Ready
                .request(WorkspaceAction::Restart)
                .unwrap(),
            WorkspaceState::Restarting
        );
    }

    #[test]
    fn provisioning_and_failed_workspaces_can_be_recovered() {
        assert_eq!(
            WorkspaceState::Provisioning
                .request(WorkspaceAction::Restart)
                .unwrap(),
            WorkspaceState::Restarting
        );
        assert_eq!(
            WorkspaceState::Provisioning
                .request(WorkspaceAction::Stop)
                .unwrap(),
            WorkspaceState::Stopping
        );
        assert_eq!(
            WorkspaceState::Failed
                .request(WorkspaceAction::Stop)
                .unwrap(),
            WorkspaceState::Stopping
        );
    }

    #[test]
    fn deleted_workspace_is_terminal() {
        for action in [
            WorkspaceAction::Start,
            WorkspaceAction::Stop,
            WorkspaceAction::Restart,
            WorkspaceAction::Delete,
        ] {
            assert!(WorkspaceState::Deleted.request(action).is_err());
        }
        for observation in [
            WorkspaceObservation::Ready,
            WorkspaceObservation::Stopped,
            WorkspaceObservation::Deleted,
            WorkspaceObservation::Failed,
        ] {
            assert!(WorkspaceState::Deleted.observe(observation).is_err());
        }
    }

    #[test]
    fn deletion_requires_cleanup_confirmation() {
        let deleting = WorkspaceState::Ready
            .request(WorkspaceAction::Delete)
            .unwrap();
        assert_eq!(deleting, WorkspaceState::Deleting);
        assert_eq!(
            deleting.observe(WorkspaceObservation::Deleted).unwrap(),
            WorkspaceState::Deleted
        );
    }
}
