use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::summary::{workspace_filter_sql, workspace_page_summary};
use super::{WorkspacePage, decode_postgres, decode_sqlite, row};
use crate::storage::{Database, StorageError};

#[derive(Debug, Clone, Deserialize, Serialize)]
struct WorkspaceCursor {
    created_at: i64,
    id: Uuid,
}

impl Database {
    pub async fn list_workspaces_page(
        &self,
        organization_id: Uuid,
        limit: Option<u32>,
        cursor: Option<&str>,
        search: Option<&str>,
    ) -> Result<WorkspacePage, StorageError> {
        let limit = i64::from(limit.unwrap_or(50).clamp(1, 200));
        let cursor = cursor
            .map(|value| {
                let bytes = URL_SAFE_NO_PAD
                    .decode(value)
                    .map_err(|_| StorageError::InvalidWorkspace)?;
                serde_json::from_slice::<WorkspaceCursor>(&bytes)
                    .map_err(|_| StorageError::InvalidWorkspace)
            })
            .transpose()?;
        // Search the immutable workspace columns and the serialized template snapshot. The
        // latter is where `workspace_user` lives, so historical workspaces use the same source of
        // truth as reconciliation. Escape LIKE metacharacters so user input remains literal.
        let search = search.unwrap_or("").trim().to_lowercase();
        let pattern = format!("%{}%", escape_like_pattern(&search));
        let summary = workspace_page_summary(self, organization_id, &search, &pattern).await?;
        let filter = workspace_filter_sql("{install}", "{organization}", "{search}", "{pattern}");
        let sql = format!(
            "SELECT {} FROM workspaces WHERE {filter} \
             AND ({{cursor_created}} IS NULL OR created_at > {{cursor_created}} OR (created_at = {{cursor_created}} AND id > {{cursor_id}})) \
             ORDER BY created_at, id LIMIT {{limit}}",
            row::WORKSPACE_COLUMNS
        );
        let rows = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => sqlx::query(
                &sql.replace("{install}", "?1")
                    .replace("{organization}", "?2")
                    .replace("{search}", "?3")
                    .replace("{pattern}", "?4")
                    .replace("{cursor_created}", "?5")
                    .replace("{cursor_id}", "?6")
                    .replace("{limit}", "?7"),
            )
            .bind(installation_id.as_str())
            .bind(organization_id.to_string())
            .bind(search)
            .bind(&pattern)
            .bind(cursor.as_ref().map(|c| c.created_at))
            .bind(cursor.as_ref().map(|c| c.id.to_string()))
            .bind(limit + 1)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| decode_sqlite(row, installation_id))
            .collect::<Result<Vec<_>, _>>()?,
            Self::Postgres {
                pool,
                installation_id,
            } => sqlx::query(
                &sql.replace("{install}", "$1")
                    .replace("{organization}", "$2")
                    .replace("{search}", "$3")
                    .replace("{pattern}", "$4")
                    .replace("{cursor_created}", "$5")
                    .replace("{cursor_id}", "$6")
                    .replace("{limit}", "$7"),
            )
            .bind(installation_id.as_str())
            .bind(organization_id.to_string())
            .bind(search)
            .bind(&pattern)
            .bind(cursor.as_ref().map(|c| c.created_at))
            .bind(cursor.as_ref().map(|c| c.id.to_string()))
            .bind(limit + 1)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| decode_postgres(row, installation_id))
            .collect::<Result<Vec<_>, _>>()?,
        };
        let mut items = rows;
        let next_cursor = if items.len() > limit as usize {
            items.pop();
            let tail = items.last().expect("page has an item");
            Some(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&WorkspaceCursor {
                created_at: tail.created_at,
                id: tail.id,
            })?))
        } else {
            None
        };
        Ok(WorkspacePage {
            items,
            next_cursor,
            total_count: summary.total_count,
            requested: summary.requested,
            state_counts: summary.state_counts,
        })
    }
}

fn escape_like_pattern(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}
