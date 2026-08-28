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
    MarkReady,
    MarkStopped,
    MarkDeleted,
    MarkFailed,
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
    pub fn transition(self, action: WorkspaceAction) -> Result<Self, TransitionError> {
        use WorkspaceAction as Action;
        use WorkspaceState as State;

        let next = match (self, action) {
            (State::Stopped | State::Failed, Action::Start) => State::Starting,
            (State::Ready, Action::Stop) => State::Stopping,
            (State::Ready, Action::Restart) => State::Restarting,
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
            (State::Provisioning | State::Starting | State::Restarting, Action::MarkReady) => {
                State::Ready
            }
            (State::Stopping, Action::MarkStopped) => State::Stopped,
            (State::Deleting, Action::MarkDeleted) => State::Deleted,
            (
                State::Provisioning
                | State::Ready
                | State::Starting
                | State::Stopping
                | State::Restarting
                | State::Deleting,
                Action::MarkFailed,
            ) => State::Failed,
            _ => {
                return Err(TransitionError {
                    state: self,
                    action,
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
            Self::MarkReady => "mark_ready",
            Self::MarkStopped => "mark_stopped",
            Self::MarkDeleted => "mark_deleted",
            Self::MarkFailed => "mark_failed",
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
#[error("workspace action {action:?} is invalid while state is {state:?}")]
pub struct TransitionError {
    pub state: WorkspaceState,
    pub action: WorkspaceAction,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_stop_and_restart_lifecycle() {
        let stopped = WorkspaceState::Ready
            .transition(WorkspaceAction::Stop)
            .unwrap()
            .transition(WorkspaceAction::MarkStopped)
            .unwrap();
        assert_eq!(stopped, WorkspaceState::Stopped);
        assert_eq!(
            stopped.transition(WorkspaceAction::Start).unwrap(),
            WorkspaceState::Starting
        );
        assert_eq!(
            WorkspaceState::Ready
                .transition(WorkspaceAction::Restart)
                .unwrap(),
            WorkspaceState::Restarting
        );
    }

    #[test]
    fn deleted_workspace_is_terminal() {
        for action in [
            WorkspaceAction::Start,
            WorkspaceAction::Stop,
            WorkspaceAction::Restart,
            WorkspaceAction::Delete,
            WorkspaceAction::MarkReady,
            WorkspaceAction::MarkStopped,
            WorkspaceAction::MarkDeleted,
            WorkspaceAction::MarkFailed,
        ] {
            assert!(WorkspaceState::Deleted.transition(action).is_err());
        }
    }

    #[test]
    fn deletion_requires_cleanup_confirmation() {
        let deleting = WorkspaceState::Ready
            .transition(WorkspaceAction::Delete)
            .unwrap();
        assert_eq!(deleting, WorkspaceState::Deleting);
        assert_eq!(
            deleting.transition(WorkspaceAction::MarkDeleted).unwrap(),
            WorkspaceState::Deleted
        );
    }
}
