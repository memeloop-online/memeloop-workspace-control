use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NewEvent {
    pub organization_id: Uuid,
    pub workspace_id: Option<Uuid>,
    pub kind: String,
    pub payload: serde_json::Value,
}

impl NewEvent {
    pub fn validate(&self) -> Result<(), EventError> {
        if self.kind.is_empty()
            || self.kind.len() > 128
            || !self
                .kind
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
        {
            return Err(EventError::InvalidKind);
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EventError {
    #[error("event kind must be 1-128 safe ASCII characters")]
    InvalidKind,
}
