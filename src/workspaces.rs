use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::quota::Resources;

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

/// A platform-owned runtime contract for a workspace image.
///
/// Profiles are deliberately an enum instead of an arbitrary pod fragment: a template may select
/// one of the runtime shapes implemented by the control plane, and every workspace stores a
/// snapshot of that selection.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceRuntimeProfile {
    #[default]
    Standard,
    CoderRustDev,
    CoderNodeDev,
    CoderTokenCenterRustDev,
    CoderClusterAdmin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Workspace {
    pub id: Uuid,
    pub short_id: String,
    pub organization_id: Uuid,
    pub owner_id: Uuid,
    pub name: String,
    pub template_id: Option<Uuid>,
    pub runtime_profile: WorkspaceRuntimeProfile,
    pub image: String,
    pub access_mode: AccessMode,
    pub state: WorkspaceState,
    pub resources: Resources,
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

impl WorkspaceRuntimeProfile {
    pub const ALL: [Self; 5] = [
        Self::Standard,
        Self::CoderRustDev,
        Self::CoderNodeDev,
        Self::CoderTokenCenterRustDev,
        Self::CoderClusterAdmin,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::CoderRustDev => "coder_rust_dev",
            Self::CoderNodeDev => "coder_node_dev",
            Self::CoderTokenCenterRustDev => "coder_token_center_rust_dev",
            Self::CoderClusterAdmin => "coder_cluster_admin",
        }
    }

    pub fn from_database(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|profile| profile.as_str() == value)
    }

    pub fn login_user(self) -> &'static str {
        match self {
            Self::Standard => "workspace",
            Self::CoderRustDev | Self::CoderTokenCenterRustDev => "rust-dev",
            Self::CoderNodeDev => "node-dev",
            Self::CoderClusterAdmin => "cluster-admin",
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

    #[test]
    fn runtime_profiles_use_stable_snake_case_names() {
        for profile in WorkspaceRuntimeProfile::ALL {
            let encoded = serde_json::to_string(&profile).unwrap();
            assert_eq!(encoded, format!("\"{}\"", profile.as_str()));
            assert_eq!(
                serde_json::from_str::<WorkspaceRuntimeProfile>(&encoded).unwrap(),
                profile
            );
            assert_eq!(
                WorkspaceRuntimeProfile::from_database(profile.as_str()),
                Some(profile)
            );
        }
        assert_eq!(
            WorkspaceRuntimeProfile::default(),
            WorkspaceRuntimeProfile::Standard
        );
        assert!(WorkspaceRuntimeProfile::from_database("arbitrary_pod_spec").is_none());
    }
}
