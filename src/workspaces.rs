use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::templates::WorkspaceTemplateSpec;

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
    #[serde(flatten)]
    pub template: WorkspaceTemplateSpec,
    pub state: WorkspaceState,
    pub generation: u64,
    pub created_at: i64,
    pub updated_at: i64,
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
