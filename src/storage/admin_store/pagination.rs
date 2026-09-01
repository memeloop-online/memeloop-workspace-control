use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::storage::{Organization, StorageError};

use super::{MembershipPage, MembershipSummary, OrganizationPage, UserPage, UserSummary};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(super) struct KeysetCursor {
    pub(super) created_at: i64,
    pub(super) id: Uuid,
}

pub(super) fn page_limit(limit: Option<u32>) -> i64 {
    i64::from(limit.unwrap_or(50).clamp(1, 200))
}

pub(super) fn decode_cursor(cursor: Option<&str>) -> Result<Option<KeysetCursor>, StorageError> {
    cursor
        .map(|cursor| {
            let bytes = URL_SAFE_NO_PAD
                .decode(cursor)
                .map_err(|_| StorageError::InvalidAuditQuery)?;
            serde_json::from_slice(&bytes).map_err(|_| StorageError::InvalidAuditQuery)
        })
        .transpose()
}

fn encode_cursor(created_at: i64, id: Uuid) -> Result<String, StorageError> {
    Ok(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&KeysetCursor { created_at, id })?))
}

pub(super) fn page_users(
    mut items: Vec<UserSummary>,
    limit: i64,
) -> Result<UserPage, StorageError> {
    let next_cursor = if items.len() > limit as usize {
        let last = items.pop().expect("page contains limit + 1 entries");
        let tail = items.last().expect("page contains at least one entry");
        debug_assert!(last.created_at >= tail.created_at);
        Some(encode_cursor(tail.created_at, tail.id)?)
    } else {
        None
    };
    Ok(UserPage { items, next_cursor })
}

pub(super) fn page_organizations(
    mut items: Vec<Organization>,
    limit: i64,
) -> Result<OrganizationPage, StorageError> {
    let next_cursor = if items.len() > limit as usize {
        items.pop();
        let tail = items.last().expect("page contains at least one entry");
        Some(encode_cursor(tail.created_at, tail.id)?)
    } else {
        None
    };
    Ok(OrganizationPage { items, next_cursor })
}

pub(super) fn page_members(
    mut items: Vec<MembershipSummary>,
    limit: i64,
) -> Result<MembershipPage, StorageError> {
    let next_cursor = if items.len() > limit as usize {
        items.pop();
        let tail = items.last().expect("page contains at least one entry");
        Some(encode_cursor(tail.user.created_at, tail.user.id)?)
    } else {
        None
    };
    Ok(MembershipPage { items, next_cursor })
}
