use sqlx::{PgConnection, PgPool, Row, SqliteConnection, SqlitePool};
use uuid::Uuid;

use crate::{
    auth::Role,
    storage::{
        Membership, Organization, StorageError,
        user_settings::{insert_key_postgres, insert_key_sqlite},
    },
};

use super::UserWithKeyCommand;

pub(super) async fn create_user_with_key_sqlite(
    pool: &SqlitePool,
    installation_id: &str,
    user_id: Uuid,
    token_hash: &str,
    command: &UserWithKeyCommand<'_>,
) -> Result<(), StorageError> {
    let mut transaction = pool.begin().await?;
    if let Some((organization_id, _)) = command.membership {
        ensure_organization_exists_sqlite(&mut transaction, installation_id, organization_id)
            .await?;
    }
    insert_user_sqlite(
        &mut transaction,
        installation_id,
        user_id,
        token_hash,
        command,
    )
    .await?;
    insert_key_sqlite(
        &mut transaction,
        installation_id,
        user_id,
        &command.initial_key,
        token_hash,
    )
    .await?;
    insert_optional_membership_sqlite(&mut transaction, installation_id, user_id, command).await?;
    transaction.commit().await?;
    Ok(())
}

pub(super) async fn create_user_with_key_postgres(
    pool: &PgPool,
    installation_id: &str,
    user_id: Uuid,
    token_hash: &str,
    command: &UserWithKeyCommand<'_>,
) -> Result<(), StorageError> {
    let mut transaction = pool.begin().await?;
    if let Some((organization_id, _)) = command.membership {
        ensure_organization_exists_postgres(&mut transaction, installation_id, organization_id)
            .await?;
    }
    insert_user_postgres(
        &mut transaction,
        installation_id,
        user_id,
        token_hash,
        command,
    )
    .await?;
    insert_key_postgres(
        &mut transaction,
        installation_id,
        user_id,
        &command.initial_key,
        token_hash,
    )
    .await?;
    insert_optional_membership_postgres(&mut transaction, installation_id, user_id, command)
        .await?;
    transaction.commit().await?;
    Ok(())
}

async fn insert_user_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    user_id: Uuid,
    token_hash: &str,
    command: &UserWithKeyCommand<'_>,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO users (id, installation_id, display_name, token_hash, \
         system_admin, disabled, created_at) VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
    )
    .bind(user_id.to_string())
    .bind(installation_id)
    .bind(command.display_name)
    .bind(token_hash)
    .bind(i64::from(command.system_admin))
    .bind(command.now)
    .execute(&mut *connection)
    .await?;
    Ok(())
}

async fn insert_user_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    user_id: Uuid,
    token_hash: &str,
    command: &UserWithKeyCommand<'_>,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO users (id, installation_id, display_name, token_hash, \
         system_admin, disabled, created_at) VALUES ($1, $2, $3, $4, $5, 0, $6)",
    )
    .bind(user_id.to_string())
    .bind(installation_id)
    .bind(command.display_name)
    .bind(token_hash)
    .bind(i64::from(command.system_admin))
    .bind(command.now)
    .execute(&mut *connection)
    .await?;
    Ok(())
}

async fn insert_optional_membership_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    user_id: Uuid,
    command: &UserWithKeyCommand<'_>,
) -> Result<(), StorageError> {
    if let Some((organization_id, role)) = command.membership {
        insert_membership_sqlite(
            connection,
            installation_id,
            organization_id,
            user_id,
            role,
            command.now,
        )
        .await?;
    }
    Ok(())
}

async fn insert_optional_membership_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    user_id: Uuid,
    command: &UserWithKeyCommand<'_>,
) -> Result<(), StorageError> {
    if let Some((organization_id, role)) = command.membership {
        insert_membership_postgres(
            connection,
            installation_id,
            organization_id,
            user_id,
            role,
            command.now,
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn ensure_organization_exists_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    organization_id: Uuid,
) -> Result<(), StorageError> {
    let found = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM organizations WHERE installation_id = ?1 AND id = ?2",
    )
    .bind(installation_id)
    .bind(organization_id.to_string())
    .fetch_one(&mut *connection)
    .await?;
    if found == 1 {
        Ok(())
    } else {
        Err(StorageError::OrganizationNotFound)
    }
}

pub(super) async fn ensure_organization_exists_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    organization_id: Uuid,
) -> Result<(), StorageError> {
    let found = sqlx::query_scalar::<_, String>(
        "SELECT id FROM organizations WHERE installation_id = $1 AND id = $2 FOR SHARE",
    )
    .bind(installation_id)
    .bind(organization_id.to_string())
    .fetch_optional(&mut *connection)
    .await?;
    if found.is_some() {
        Ok(())
    } else {
        Err(StorageError::OrganizationNotFound)
    }
}

pub(super) async fn insert_membership_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    organization_id: Uuid,
    user_id: Uuid,
    role: Role,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO organization_memberships (installation_id, organization_id, user_id, role, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(installation_id)
    .bind(organization_id.to_string())
    .bind(user_id.to_string())
    .bind(role.as_str())
    .bind(now)
    .execute(&mut *connection)
    .await?;
    Ok(())
}

pub(super) async fn insert_membership_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    organization_id: Uuid,
    user_id: Uuid,
    role: Role,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO organization_memberships (installation_id, organization_id, user_id, role, created_at) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(installation_id)
    .bind(organization_id.to_string())
    .bind(user_id.to_string())
    .bind(role.as_str())
    .bind(now)
    .execute(&mut *connection)
    .await?;
    Ok(())
}

pub(super) async fn insert_organization_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    organization: &Organization,
    owner_user_id: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO organizations (id, installation_id, name, created_at) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(organization.id.to_string())
    .bind(installation_id)
    .bind(&organization.name)
    .bind(now)
    .execute(&mut *connection)
    .await?;
    sqlx::query("INSERT INTO organization_memberships (installation_id, organization_id, user_id, role, created_at) VALUES (?1, ?2, ?3, 'organization_admin', ?4)")
        .bind(installation_id).bind(organization.id.to_string()).bind(owner_user_id.to_string())
        .bind(now).execute(&mut *connection).await?;
    Ok(())
}

pub(super) async fn insert_organization_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    organization: &Organization,
    owner_user_id: Uuid,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO organizations (id, installation_id, name, created_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(organization.id.to_string())
    .bind(installation_id)
    .bind(&organization.name)
    .bind(now)
    .execute(&mut *connection)
    .await?;
    sqlx::query("INSERT INTO organization_memberships (installation_id, organization_id, user_id, role, created_at) VALUES ($1, $2, $3, 'organization_admin', $4)")
        .bind(installation_id).bind(organization.id.to_string()).bind(owner_user_id.to_string())
        .bind(now).execute(&mut *connection).await?;
    Ok(())
}

pub(super) async fn sqlite_memberships(
    pool: &sqlx::SqlitePool,
    installation_id: &str,
    user_id: Uuid,
) -> Result<Vec<Membership>, StorageError> {
    let rows = sqlx::query("SELECT organization_id, role FROM organization_memberships WHERE installation_id = ?1 AND user_id = ?2")
        .bind(installation_id).bind(user_id.to_string()).fetch_all(pool).await?;
    decode_memberships(
        rows.iter()
            .map(|row| (row.try_get("organization_id"), row.try_get("role"))),
    )
}

pub(super) async fn postgres_memberships(
    pool: &sqlx::PgPool,
    installation_id: &str,
    user_id: Uuid,
) -> Result<Vec<Membership>, StorageError> {
    let rows = sqlx::query("SELECT organization_id, role FROM organization_memberships WHERE installation_id = $1 AND user_id = $2")
        .bind(installation_id).bind(user_id.to_string()).fetch_all(pool).await?;
    decode_memberships(
        rows.iter()
            .map(|row| (row.try_get("organization_id"), row.try_get("role"))),
    )
}

fn decode_memberships<I>(rows: I) -> Result<Vec<Membership>, StorageError>
where
    I: IntoIterator<Item = (Result<String, sqlx::Error>, Result<String, sqlx::Error>)>,
{
    rows.into_iter()
        .map(|(organization_id, role)| {
            let role = role?;
            Ok(Membership {
                organization_id: Uuid::parse_str(&organization_id?)?,
                role: Role::from_database(&role).ok_or(StorageError::UnknownRole(role))?,
            })
        })
        .collect()
}

pub(super) async fn mark_key_used_sqlite(
    pool: &sqlx::SqlitePool,
    installation: &str,
    key_id: &str,
    last_used_at: Option<i64>,
) -> Result<(), StorageError> {
    let now = current_timestamp()?;
    if !last_used_at.is_none_or(|value| value < now.saturating_sub(300)) {
        return Ok(());
    }
    sqlx::query("UPDATE user_api_keys SET last_used_at = ?1 WHERE installation_id = ?2 AND id = ?3 AND (last_used_at IS NULL OR last_used_at < ?4)")
        .bind(now).bind(installation).bind(key_id).bind(now.saturating_sub(300))
        .execute(pool).await?;
    Ok(())
}

pub(super) async fn mark_key_used_postgres(
    pool: &sqlx::PgPool,
    installation: &str,
    key_id: &str,
    last_used_at: Option<i64>,
) -> Result<(), StorageError> {
    let now = current_timestamp()?;
    if !last_used_at.is_none_or(|value| value < now.saturating_sub(300)) {
        return Ok(());
    }
    sqlx::query("UPDATE user_api_keys SET last_used_at = $1 WHERE installation_id = $2 AND id = $3 AND (last_used_at IS NULL OR last_used_at < $4)")
        .bind(now).bind(installation).bind(key_id).bind(now.saturating_sub(300))
        .execute(pool).await?;
    Ok(())
}

fn current_timestamp() -> Result<i64, StorageError> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| StorageError::Clock)?
        .as_secs();
    i64::try_from(seconds).map_err(|_| StorageError::Clock)
}
