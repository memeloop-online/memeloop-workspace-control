use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::summary::{workspace_filter_sql, workspace_page_summary};
use super::{WorkspacePage, decode_postgres, decode_sqlite, row, template_filter_sql};
use crate::storage::{Database, StorageError};

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceSort {
    #[default]
    Newest,
    Oldest,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct WorkspaceCursor {
    created_at: i64,
    id: Uuid,
    sort: WorkspaceSort,
}

struct PageQuery<'a> {
    organization_id: Uuid,
    search: &'a str,
    pattern: &'a str,
    cursor: Option<&'a WorkspaceCursor>,
    sort: WorkspaceSort,
    limit: i64,
    allowed_template_ids: Option<&'a [Uuid]>,
}

impl PageQuery<'_> {
    fn sql(&self, placeholder: char) -> String {
        let filter = workspace_filter_sql(
            &format!("{placeholder}1"),
            &format!("{placeholder}2"),
            &format!("{placeholder}3"),
            &format!("{placeholder}4"),
        );
        let templates = template_filter_sql(self.allowed_template_ids, placeholder, 8);
        let (comparison, direction) = match self.sort {
            WorkspaceSort::Newest => ("<", "DESC"),
            WorkspaceSort::Oldest => (">", "ASC"),
        };
        format!(
            "SELECT {} FROM workspaces WHERE {filter}{templates} \
             AND ({placeholder}5 IS NULL OR created_at {comparison} {placeholder}5 OR (created_at = {placeholder}5 AND id {comparison} {placeholder}6)) \
             ORDER BY created_at {direction}, id {direction} LIMIT {placeholder}7",
            row::WORKSPACE_COLUMNS
        )
    }
}

impl Database {
    pub async fn list_workspaces_page(
        &self,
        organization_id: Uuid,
        limit: Option<u32>,
        cursor: Option<&str>,
        search: Option<&str>,
        sort: WorkspaceSort,
        allowed_template_ids: Option<&[Uuid]>,
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
        if cursor.as_ref().is_some_and(|cursor| cursor.sort != sort) {
            return Err(StorageError::InvalidWorkspace);
        }
        // Search the immutable workspace columns and the serialized template snapshot. The
        // latter is where `workspace_user` lives, so historical workspaces use the same source of
        // truth as reconciliation. Escape LIKE metacharacters so user input remains literal.
        let search = search.unwrap_or("").trim().to_lowercase();
        let pattern = format!("%{}%", escape_like_pattern(&search));
        let summary = workspace_page_summary(
            self,
            organization_id,
            &search,
            &pattern,
            allowed_template_ids,
        )
        .await?;
        let mut items = self
            .read_workspace_page(PageQuery {
                organization_id,
                search: &search,
                pattern: &pattern,
                cursor: cursor.as_ref(),
                sort,
                limit: limit + 1,
                allowed_template_ids,
            })
            .await?;
        let next_cursor = if items.len() > limit as usize {
            items.pop();
            let tail = items.last().expect("page has an item");
            Some(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&WorkspaceCursor {
                created_at: tail.created_at,
                id: tail.id,
                sort,
            })?))
        } else {
            None
        };
        Ok(WorkspacePage {
            items,
            next_cursor,
            total_count: summary.total_count,
            requested: summary.requested,
            temporary_requested_gib: summary.requested.temporary_storage_gib,
            state_counts: summary.state_counts,
        })
    }
    async fn read_workspace_page(
        &self,
        page: PageQuery<'_>,
    ) -> Result<Vec<crate::workspaces::Workspace>, StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let sql = page.sql('?');
                let mut query = sqlx::query(&sql)
                    .bind(installation_id.as_str())
                    .bind(page.organization_id.to_string())
                    .bind(page.search)
                    .bind(page.pattern)
                    .bind(page.cursor.map(|c| c.created_at))
                    .bind(page.cursor.map(|c| c.id.to_string()))
                    .bind(page.limit);
                for id in page.allowed_template_ids.unwrap_or_default() {
                    query = query.bind(id.to_string());
                }
                query
                    .fetch_all(pool)
                    .await?
                    .into_iter()
                    .map(|row| decode_sqlite(row, installation_id))
                    .collect()
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let sql = page.sql('$');
                let mut query = sqlx::query(&sql)
                    .bind(installation_id.as_str())
                    .bind(page.organization_id.to_string())
                    .bind(page.search)
                    .bind(page.pattern)
                    .bind(page.cursor.map(|c| c.created_at))
                    .bind(page.cursor.map(|c| c.id.to_string()))
                    .bind(page.limit);
                for id in page.allowed_template_ids.unwrap_or_default() {
                    query = query.bind(id.to_string());
                }
                query
                    .fetch_all(pool)
                    .await?
                    .into_iter()
                    .map(|row| decode_postgres(row, installation_id))
                    .collect()
            }
        }
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
