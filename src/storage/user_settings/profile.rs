use base64::{Engine, engine::general_purpose::STANDARD};
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
        .map(validate_avatar_data_url)
        .transpose()?;
    Ok((display_name.to_owned(), avatar_url))
}

const MAX_AVATAR_BYTES: usize = 512 * 1024;

fn validate_avatar_data_url(value: &str) -> Result<String, StorageError> {
    if value.len() > 4 * MAX_AVATAR_BYTES.div_ceil(3) + 32 || value.chars().any(char::is_control) {
        return Err(StorageError::InvalidUserProfile);
    }
    let (media_type, payload) = value
        .strip_prefix("data:")
        .and_then(|value| value.split_once(";base64,"))
        .ok_or(StorageError::InvalidUserProfile)?;
    if !matches!(media_type, "image/png" | "image/jpeg" | "image/webp") || payload.is_empty() {
        return Err(StorageError::InvalidUserProfile);
    }
    let bytes = STANDARD
        .decode(payload)
        .map_err(|_| StorageError::InvalidUserProfile)?;
    if bytes.len() > MAX_AVATAR_BYTES || !matches_magic(media_type, &bytes) {
        return Err(StorageError::InvalidUserProfile);
    }
    // Canonical base64 prevents alternate spellings and keeps the stored payload stable.
    Ok(format!(
        "data:{media_type};base64,{}",
        STANDARD.encode(bytes)
    ))
}

fn matches_magic(media_type: &str, bytes: &[u8]) -> bool {
    match media_type {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/webp" => bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::validate_avatar_data_url;
    use base64::{Engine, engine::general_purpose::STANDARD};

    #[test]
    fn only_local_raster_data_urls_are_accepted() {
        let png = format!(
            "data:image/png;base64,{}",
            STANDARD.encode(b"\x89PNG\r\n\x1a\nbody")
        );
        assert_eq!(validate_avatar_data_url(&png).unwrap(), png);
        for invalid in [
            "https://example.test/a.png",
            "data:image/svg+xml;base64,PHN2Zy8+",
            "data:image/png;base64,not base64",
            "data:image/jpeg;base64,iVBORw0KGgo=",
        ] {
            assert!(validate_avatar_data_url(invalid).is_err(), "{invalid}");
        }
    }
}
