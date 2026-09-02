use uuid::Uuid;

use crate::storage::StorageError;

/// Serializes membership mutations and account disabling that could change the active
/// organization-administrator set. All callers must use this exact key.
pub(super) async fn lock_organization_membership_writes_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    installation_id: &str,
    organization_id: Uuid,
) -> Result<(), StorageError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!(
            "mwc:organization-membership:{installation_id}:{organization_id}"
        ))
        .execute(&mut **tx)
        .await?;
    Ok(())
}
