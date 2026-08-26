use sqlx::{PgConnection, Row, SqliteConnection};
use uuid::Uuid;

use super::{Database, StorageError};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceInjectionRefs {
    pub organization: Option<Vec<String>>,
    pub user: Option<Vec<String>>,
}

impl WorkspaceInjectionRefs {
    pub(super) fn validate(&self) -> Result<(), StorageError> {
        for refs in [&self.organization, &self.user].into_iter().flatten() {
            let mut sorted = refs.clone();
            sorted.sort();
            if sorted.windows(2).any(|pair| pair[0] == pair[1])
                || sorted.iter().any(|key| {
                    key.is_empty()
                        || key.len() > 128
                        || !key.chars().all(|character| {
                            character.is_ascii_alphanumeric() || "._-".contains(character)
                        })
                })
            {
                return Err(StorageError::InvalidWorkspaceInjectionRefs);
            }
        }
        Ok(())
    }
}

impl Database {
    pub async fn workspace_injection_refs(
        &self,
        workspace_id: Uuid,
    ) -> Result<WorkspaceInjectionRefs, StorageError> {
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let rows = sqlx::query("SELECT scope, injection_key FROM workspace_injection_refs WHERE installation_id = ?1 AND workspace_id = ?2 ORDER BY scope, injection_key")
                    .bind(installation_id.as_str()).bind(workspace_id.to_string()).fetch_all(pool).await?;
                decode_rows(rows)
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                let rows = sqlx::query("SELECT scope, injection_key FROM workspace_injection_refs WHERE installation_id = $1 AND workspace_id = $2 ORDER BY scope, injection_key")
                    .bind(installation_id.as_str()).bind(workspace_id.to_string()).fetch_all(pool).await?;
                decode_rows(rows)
            }
        }
    }
}

fn decode_rows<R: Row>(rows: Vec<R>) -> Result<WorkspaceInjectionRefs, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'decode> sqlx::Decode<'decode, R::Database> + sqlx::Type<R::Database>,
{
    let mut organization = Vec::new();
    let mut user = Vec::new();
    for row in rows {
        let scope: String = row.try_get("scope")?;
        let key: String = row.try_get("injection_key")?;
        match scope.as_str() {
            "organization" => organization.push(key),
            "user" => user.push(key),
            _ => return Err(StorageError::InvalidWorkspaceInjectionRefs),
        }
    }
    Ok(WorkspaceInjectionRefs {
        organization: decode_scope(organization)?,
        user: decode_scope(user)?,
    })
}

pub(super) async fn insert_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace_id: Uuid,
    refs: &WorkspaceInjectionRefs,
    now: i64,
) -> Result<(), StorageError> {
    refs.validate()?;
    for (scope, keys) in [("organization", &refs.organization), ("user", &refs.user)] {
        for key in encode_scope(keys) {
            sqlx::query("INSERT INTO workspace_injection_refs (installation_id, workspace_id, scope, injection_key, created_at) VALUES (?1, ?2, ?3, ?4, ?5)")
                .bind(installation_id).bind(workspace_id.to_string()).bind(scope).bind(key).bind(now)
                .execute(&mut *connection).await?;
        }
    }
    Ok(())
}

pub(super) async fn insert_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace_id: Uuid,
    refs: &WorkspaceInjectionRefs,
    now: i64,
) -> Result<(), StorageError> {
    refs.validate()?;
    for (scope, keys) in [("organization", &refs.organization), ("user", &refs.user)] {
        for key in encode_scope(keys) {
            sqlx::query("INSERT INTO workspace_injection_refs (installation_id, workspace_id, scope, injection_key, created_at) VALUES ($1, $2, $3, $4, $5)")
                .bind(installation_id).bind(workspace_id.to_string()).bind(scope).bind(key).bind(now)
                .execute(&mut *connection).await?;
        }
    }
    Ok(())
}

fn encode_scope(refs: &Option<Vec<String>>) -> Vec<&str> {
    match refs {
        None => vec!["*"],
        Some(keys) if keys.is_empty() => vec!["!"],
        Some(keys) => keys.iter().map(String::as_str).collect(),
    }
}

fn decode_scope(mut keys: Vec<String>) -> Result<Option<Vec<String>>, StorageError> {
    if keys.is_empty() || keys == ["*"] {
        return Ok(None);
    }
    if keys == ["!"] {
        return Ok(Some(Vec::new()));
    }
    if keys.iter().any(|key| key == "*" || key == "!") {
        return Err(StorageError::InvalidWorkspaceInjectionRefs);
    }
    keys.sort();
    Ok(Some(keys))
}
