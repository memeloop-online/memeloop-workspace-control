use sqlx::{Postgres, Sqlite, Transaction};

use crate::storage::StorageError;

/// Adds the optional API-key template allowlist. NULL is intentionally used
/// for legacy/unrestricted keys so an empty JSON array remains meaningful.
pub(super) async fn upgrade_sqlite(
    transaction: &mut Transaction<'_, Sqlite>,
) -> Result<(), StorageError> {
    sqlx::query("ALTER TABLE user_api_keys ADD COLUMN allowed_template_ids_json TEXT")
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

pub(super) async fn upgrade_postgres(
    transaction: &mut Transaction<'_, Postgres>,
) -> Result<(), StorageError> {
    sqlx::query("ALTER TABLE user_api_keys ADD COLUMN allowed_template_ids_json TEXT")
        .execute(&mut **transaction)
        .await?;
    Ok(())
}
