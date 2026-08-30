use sqlx::Row;
use uuid::Uuid;

use super::super::{Database, StorageError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredUserProfile {
    pub user_id: Uuid,
    pub display_name: String,
    pub avatar_url: Option<String>,
}

impl Database {
    pub async fn get_user_profile(&self, user_id: Uuid) -> Result<StoredUserProfile, StorageError> {
        let row = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                let row = sqlx::query(
                    "SELECT display_name, avatar_url FROM users WHERE installation_id = ?1 AND id = ?2 AND disabled = 0",
                )
                .bind(installation_id.as_str())
                .bind(user_id.to_string())
                .fetch_optional(pool)
                .await?;
                return row
                    .map(|row| decode_profile(&row, user_id))
                    .transpose()?
                    .ok_or(StorageError::UserNotFound);
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query(
                    "SELECT display_name, avatar_url FROM users WHERE installation_id = $1 AND id = $2 AND disabled = 0",
                )
                .bind(installation_id.as_str())
                .bind(user_id.to_string())
                .fetch_optional(pool)
                .await?
            }
        };
        row.map(|row| decode_profile(&row, user_id))
            .transpose()?
            .ok_or(StorageError::UserNotFound)
    }

    pub async fn update_user_profile(
        &self,
        user_id: Uuid,
        display_name: &str,
        avatar_url: Option<&str>,
    ) -> Result<StoredUserProfile, StorageError> {
        let (display_name, avatar_url) = validate_profile(display_name, avatar_url)?;
        let rows = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query(
                    "UPDATE users SET display_name = ?1, avatar_url = ?2 WHERE installation_id = ?3 AND id = ?4 AND disabled = 0",
                )
                .bind(&display_name)
                .bind(&avatar_url)
                .bind(installation_id.as_str())
                .bind(user_id.to_string())
                .execute(pool)
                .await?
                .rows_affected()
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query(
                    "UPDATE users SET display_name = $1, avatar_url = $2 WHERE installation_id = $3 AND id = $4 AND disabled = 0",
                )
                .bind(&display_name)
                .bind(&avatar_url)
                .bind(installation_id.as_str())
                .bind(user_id.to_string())
                .execute(pool)
                .await?
                .rows_affected()
            }
        };
        if rows != 1 {
            return Err(StorageError::UserNotFound);
        }
        Ok(StoredUserProfile {
            user_id,
            display_name,
            avatar_url,
        })
    }
}

fn decode_profile<R: Row>(row: &R, user_id: Uuid) -> Result<StoredUserProfile, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    Ok(StoredUserProfile {
        user_id,
        display_name: row.try_get("display_name")?,
        avatar_url: row.try_get("avatar_url")?,
    })
}

fn validate_profile(
    display_name: &str,
    avatar_url: Option<&str>,
) -> Result<(String, Option<String>), StorageError> {
    let display_name = display_name.trim();
    if display_name.is_empty()
        || display_name.chars().count() > 80
        || display_name.chars().any(char::is_control)
    {
        return Err(StorageError::InvalidUserProfile);
    }
    let avatar_url = avatar_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(validate_avatar_url)
        .transpose()?;
    Ok((display_name.to_owned(), avatar_url))
}

fn validate_avatar_url(value: &str) -> Result<String, StorageError> {
    if value.len() > 2_048 || value.chars().any(char::is_control) {
        return Err(StorageError::InvalidUserProfile);
    }
    let url = url::Url::parse(value).map_err(|_| StorageError::InvalidUserProfile)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(StorageError::InvalidUserProfile);
    }
    Ok(url.to_string())
}
