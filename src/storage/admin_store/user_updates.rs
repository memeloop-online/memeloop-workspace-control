use sqlx::{Row, postgres::PgRow, sqlite::SqliteRow};
use uuid::Uuid;

use crate::{auth::Role, storage::StorageError};

use super::UserSummary;
use super::organization_locks::lock_organization_membership_writes_postgres;

pub(super) fn escape_like_pattern(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

pub(super) async fn update_user_sqlite(
    pool: &sqlx::SqlitePool,
    installation_id: &str,
    user_id: Uuid,
    display_name: Option<&str>,
    system_admin: Option<bool>,
    disabled: Option<bool>,
) -> Result<UserSummary, StorageError> {
    // `organization_memberships` mutations also use `BEGIN IMMEDIATE`. Acquire the SQLite write
    // lock before reading the user and its memberships so disabling two administrators (or
    // changing a membership concurrently) cannot leave an organization without an administrator.
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let current = sqlx::query("SELECT id, display_name, system_admin, disabled, created_at FROM users WHERE installation_id = ?1 AND id = ?2")
        .bind(installation_id)
        .bind(user_id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        .map(decode_sqlite_user)
        .transpose()?
        .ok_or(StorageError::UserNotFound)?;
    let next_admin = system_admin.unwrap_or(current.system_admin);
    let next_disabled = disabled.unwrap_or(current.disabled);
    if current.system_admin && !current.disabled && (!next_admin || next_disabled) {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE installation_id = ?1 AND system_admin = 1 AND disabled = 0")
            .bind(installation_id)
            .fetch_one(&mut *tx)
            .await?;
        if count <= 1 {
            return Err(StorageError::LastSystemAdmin);
        }
    }
    if !current.disabled && next_disabled {
        ensure_user_keeps_active_organization_administrator_sqlite(
            &mut tx,
            installation_id,
            user_id,
        )
        .await?;
    }
    let next_display_name = display_name.unwrap_or(&current.display_name).trim();
    persist_user_update_sqlite(
        &mut tx,
        installation_id,
        user_id,
        next_display_name,
        next_admin,
        next_disabled,
    )
    .await?;
    tx.commit().await?;
    Ok(updated_user_summary(
        current,
        display_name,
        next_admin,
        next_disabled,
    ))
}

async fn persist_user_update_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    installation_id: &str,
    user_id: Uuid,
    display_name: &str,
    next_admin: bool,
    next_disabled: bool,
) -> Result<(), StorageError> {
    sqlx::query("UPDATE users SET display_name = ?1, system_admin = ?2, disabled = ?3 WHERE installation_id = ?4 AND id = ?5")
        .bind(display_name)
        .bind(i64::from(next_admin))
        .bind(i64::from(next_disabled))
        .bind(installation_id)
        .bind(user_id.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(super) async fn update_user_postgres(
    pool: &sqlx::PgPool,
    installation_id: &str,
    user_id: Uuid,
    display_name: Option<&str>,
    system_admin: Option<bool>,
    disabled: Option<bool>,
) -> Result<UserSummary, StorageError> {
    let mut tx = pool.begin().await?;
    // Serialize all changes that could affect the last active system administrator for this
    // installation. Row locks alone do not protect concurrent demotions of different rows.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("mwc:active-system-admins:{installation_id}"))
        .execute(&mut *tx)
        .await?;
    let current = sqlx::query("SELECT id, display_name, system_admin, disabled, created_at FROM users WHERE installation_id = $1 AND id = $2 FOR UPDATE")
        .bind(installation_id)
        .bind(user_id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        .map(decode_postgres_user)
        .transpose()?
        .ok_or(StorageError::UserNotFound)?;
    let next_admin = system_admin.unwrap_or(current.system_admin);
    let next_disabled = disabled.unwrap_or(current.disabled);
    if current.system_admin && !current.disabled && (!next_admin || next_disabled) {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE installation_id = $1 AND system_admin = 1 AND disabled = 0")
            .bind(installation_id)
            .fetch_one(&mut *tx)
            .await?;
        if count <= 1 {
            return Err(StorageError::LastSystemAdmin);
        }
    }
    if !current.disabled && next_disabled {
        let organization_ids: Vec<String> = sqlx::query_scalar(
            "SELECT organization_id FROM organization_memberships \
             WHERE installation_id = $1 AND user_id = $2 AND role = 'organization_admin' \
             ORDER BY organization_id",
        )
        .bind(installation_id)
        .bind(user_id.to_string())
        .fetch_all(&mut *tx)
        .await?;
        // Membership mutations take this exact per-organization lock before their guard query.
        // Lock in a stable order before observing the active-admin set, so a membership demotion
        // and account disable cannot both commit and leave an organization unmanaged.
        for organization_id in organization_ids {
            lock_organization_membership_writes_postgres(
                &mut tx,
                installation_id,
                Uuid::parse_str(&organization_id)?,
            )
            .await?;
        }
        ensure_user_keeps_active_organization_administrator_postgres(
            &mut tx,
            installation_id,
            user_id,
        )
        .await?;
    }
    let next_display_name = display_name.unwrap_or(&current.display_name).trim();
    persist_user_update_postgres(
        &mut tx,
        installation_id,
        user_id,
        next_display_name,
        next_admin,
        next_disabled,
    )
    .await?;
    tx.commit().await?;
    Ok(updated_user_summary(
        current,
        display_name,
        next_admin,
        next_disabled,
    ))
}

async fn persist_user_update_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    installation_id: &str,
    user_id: Uuid,
    display_name: &str,
    next_admin: bool,
    next_disabled: bool,
) -> Result<(), StorageError> {
    sqlx::query("UPDATE users SET display_name = $1, system_admin = $2, disabled = $3 WHERE installation_id = $4 AND id = $5")
        .bind(display_name)
        .bind(i64::from(next_admin))
        .bind(i64::from(next_disabled))
        .bind(installation_id)
        .bind(user_id.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn updated_user_summary(
    mut current: UserSummary,
    display_name: Option<&str>,
    next_admin: bool,
    next_disabled: bool,
) -> UserSummary {
    current.display_name = display_name
        .unwrap_or(&current.display_name)
        .trim()
        .to_owned();
    current.system_admin = next_admin;
    current.disabled = next_disabled;
    current.membership_role = None;
    current
}

async fn ensure_user_keeps_active_organization_administrator_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    installation_id: &str,
    user_id: Uuid,
) -> Result<(), StorageError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) \
         FROM organization_memberships current_membership \
         WHERE current_membership.installation_id = ?1 \
           AND current_membership.user_id = ?2 \
           AND current_membership.role = 'organization_admin' \
           AND NOT EXISTS ( \
               SELECT 1 \
               FROM organization_memberships other_membership \
               JOIN users other_user \
                 ON other_user.installation_id = other_membership.installation_id \
                AND other_user.id = other_membership.user_id \
               WHERE other_membership.installation_id = current_membership.installation_id \
                 AND other_membership.organization_id = current_membership.organization_id \
                 AND other_membership.role = 'organization_admin' \
                 AND other_user.disabled = 0 \
                 AND other_user.id <> ?2 \
           )",
    )
    .bind(installation_id)
    .bind(user_id.to_string())
    .fetch_one(&mut **tx)
    .await?;
    if count > 0 {
        return Err(StorageError::LastOrganizationAdmin);
    }
    Ok(())
}

async fn ensure_user_keeps_active_organization_administrator_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    installation_id: &str,
    user_id: Uuid,
) -> Result<(), StorageError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) \
         FROM organization_memberships current_membership \
         WHERE current_membership.installation_id = $1 \
           AND current_membership.user_id = $2 \
           AND current_membership.role = 'organization_admin' \
           AND NOT EXISTS ( \
               SELECT 1 \
               FROM organization_memberships other_membership \
               JOIN users other_user \
                 ON other_user.installation_id = other_membership.installation_id \
                AND other_user.id = other_membership.user_id \
               WHERE other_membership.installation_id = current_membership.installation_id \
                 AND other_membership.organization_id = current_membership.organization_id \
                 AND other_membership.role = 'organization_admin' \
                 AND other_user.disabled = 0 \
                 AND other_user.id <> $2 \
           )",
    )
    .bind(installation_id)
    .bind(user_id.to_string())
    .fetch_one(&mut **tx)
    .await?;
    if count > 0 {
        return Err(StorageError::LastOrganizationAdmin);
    }
    Ok(())
}

pub(super) fn decode_sqlite_user(row: SqliteRow) -> Result<UserSummary, StorageError> {
    decode_user(&row)
}

pub(super) fn decode_postgres_user(row: PgRow) -> Result<UserSummary, StorageError> {
    decode_user(&row)
}

pub(super) fn decode_sqlite_user_with_membership(
    row: SqliteRow,
) -> Result<UserSummary, StorageError> {
    decode_user_with_membership(&row)
}

pub(super) fn decode_postgres_user_with_membership(
    row: PgRow,
) -> Result<UserSummary, StorageError> {
    decode_user_with_membership(&row)
}

pub(super) fn decode_user<R: Row>(row: &R) -> Result<UserSummary, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    Ok(UserSummary {
        id: Uuid::parse_str(&row.try_get::<String, _>("id")?)?,
        display_name: row.try_get("display_name")?,
        system_admin: row.try_get::<i64, _>("system_admin")? != 0,
        disabled: row.try_get::<i64, _>("disabled")? != 0,
        created_at: row.try_get("created_at")?,
        membership_role: None,
    })
}

fn decode_user_with_membership<R: Row>(row: &R) -> Result<UserSummary, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    let mut user = decode_user(row)?;
    user.membership_role = Some(
        row.try_get::<Option<String>, _>("membership_role")?
            .map(|role| Role::from_database(&role).ok_or(StorageError::UnknownRole(role)))
            .transpose()?,
    );
    Ok(user)
}
