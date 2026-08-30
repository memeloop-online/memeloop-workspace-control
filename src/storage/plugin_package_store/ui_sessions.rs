use sqlx::Row;
use uuid::Uuid;

use super::{CreatePluginUiSession, Database, PluginUiSession, StorageError};

impl Database {
    pub async fn create_plugin_ui_session(
        &self,
        input: CreatePluginUiSession<'_>,
    ) -> Result<PluginUiSession, StorageError> {
        let id = Uuid::now_v7();
        let methods = serde_json::to_string(input.allowed_bridge_methods)?;
        let active: i64 = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query(
                    "DELETE FROM plugin_ui_sessions WHERE installation_id=?1 AND expires_at<=?2",
                )
                .bind(installation_id.as_str())
                .bind(input.now)
                .execute(pool)
                .await?;
                sqlx::query_scalar("SELECT COUNT(*) FROM plugin_ui_sessions WHERE installation_id=?1 AND user_id=?2").bind(installation_id.as_str()).bind(input.user_id.to_string()).fetch_one(pool).await?
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query(
                    "DELETE FROM plugin_ui_sessions WHERE installation_id=$1 AND expires_at<=$2",
                )
                .bind(installation_id.as_str())
                .bind(input.now)
                .execute(pool)
                .await?;
                sqlx::query_scalar("SELECT COUNT(*) FROM plugin_ui_sessions WHERE installation_id=$1 AND user_id=$2").bind(installation_id.as_str()).bind(input.user_id.to_string()).fetch_one(pool).await?
            }
        };
        if active >= 32 {
            return Err(StorageError::PluginCapacityExceeded);
        }
        let changed=match self{
            Self::Sqlite{pool,installation_id}=>sqlx::query("INSERT INTO plugin_ui_sessions (id, installation_id, plugin_id, surface_id, user_id, ticket_hash, cookie_hash, channel_nonce, allowed_bridge_methods_json, entrypoint, package_digest, expires_at, consumed_at, created_at) SELECT ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,NULL,?13 WHERE EXISTS (SELECT 1 FROM plugin_packages WHERE installation_id=?2 AND plugin_id=?3 AND enabled=1 AND package_digest=?11)").bind(id.to_string()).bind(installation_id.as_str()).bind(input.plugin_id).bind(input.surface_id).bind(input.user_id.to_string()).bind(input.ticket_hash).bind(input.cookie_hash).bind(input.channel_nonce).bind(methods).bind(input.entrypoint).bind(input.package_digest).bind(input.expires_at).bind(input.now).execute(pool).await?.rows_affected(),
            Self::Postgres{pool,installation_id}=>sqlx::query("INSERT INTO plugin_ui_sessions (id, installation_id, plugin_id, surface_id, user_id, ticket_hash, cookie_hash, channel_nonce, allowed_bridge_methods_json, entrypoint, package_digest, expires_at, consumed_at, created_at) SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,NULL,$13 WHERE EXISTS (SELECT 1 FROM plugin_packages WHERE installation_id=$2 AND plugin_id=$3 AND enabled=1 AND package_digest=$11)").bind(id.to_string()).bind(installation_id.as_str()).bind(input.plugin_id).bind(input.surface_id).bind(input.user_id.to_string()).bind(input.ticket_hash).bind(input.cookie_hash).bind(input.channel_nonce).bind(methods).bind(input.entrypoint).bind(input.package_digest).bind(input.expires_at).bind(input.now).execute(pool).await?.rows_affected()};
        if changed != 1 {
            return Err(StorageError::PluginPackageNotFound);
        }
        Ok(PluginUiSession {
            id,
            plugin_id: input.plugin_id.to_owned(),
            surface_id: input.surface_id.to_owned(),
            user_id: input.user_id,
            entrypoint: input.entrypoint.to_owned(),
            package_digest: input.package_digest.to_owned(),
            channel_nonce: input.channel_nonce.to_owned(),
            allowed_bridge_methods: input.allowed_bridge_methods.to_vec(),
            expires_at: input.expires_at,
        })
    }

    pub async fn consume_plugin_ui_ticket(
        &self,
        ticket_hash: &str,
        now: i64,
    ) -> Result<PluginUiSession, StorageError> {
        const C: &str = "id, plugin_id, surface_id, user_id, entrypoint, package_digest, channel_nonce, allowed_bridge_methods_json, expires_at";
        match self{Self::Sqlite{pool,installation_id}=>sqlx::query(&format!("UPDATE plugin_ui_sessions SET consumed_at=?1 WHERE installation_id=?2 AND ticket_hash=?3 AND consumed_at IS NULL AND expires_at>?1 RETURNING {C}")).bind(now).bind(installation_id.as_str()).bind(ticket_hash).fetch_optional(pool).await?.map(decode).transpose()?.ok_or(StorageError::PluginUiSessionInvalid),Self::Postgres{pool,installation_id}=>sqlx::query(&format!("UPDATE plugin_ui_sessions SET consumed_at=$1 WHERE installation_id=$2 AND ticket_hash=$3 AND consumed_at IS NULL AND expires_at>$1 RETURNING {C}")).bind(now).bind(installation_id.as_str()).bind(ticket_hash).fetch_optional(pool).await?.map(decode).transpose()?.ok_or(StorageError::PluginUiSessionInvalid)}
    }

    pub async fn plugin_ui_session_by_cookie(
        &self,
        id: Uuid,
        cookie_hash: &str,
        now: i64,
    ) -> Result<PluginUiSession, StorageError> {
        const C: &str = "id, plugin_id, surface_id, user_id, entrypoint, package_digest, channel_nonce, allowed_bridge_methods_json, expires_at";
        match self{Self::Sqlite{pool,installation_id}=>sqlx::query(&format!("SELECT {C} FROM plugin_ui_sessions WHERE installation_id=?1 AND id=?2 AND cookie_hash=?3 AND consumed_at IS NOT NULL AND expires_at>?4")).bind(installation_id.as_str()).bind(id.to_string()).bind(cookie_hash).bind(now).fetch_optional(pool).await?.map(decode).transpose()?.ok_or(StorageError::PluginUiSessionInvalid),Self::Postgres{pool,installation_id}=>sqlx::query(&format!("SELECT {C} FROM plugin_ui_sessions WHERE installation_id=$1 AND id=$2 AND cookie_hash=$3 AND consumed_at IS NOT NULL AND expires_at>$4")).bind(installation_id.as_str()).bind(id.to_string()).bind(cookie_hash).bind(now).fetch_optional(pool).await?.map(decode).transpose()?.ok_or(StorageError::PluginUiSessionInvalid)}
    }
}

fn decode<R: Row>(row: R) -> Result<PluginUiSession, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    Ok(PluginUiSession {
        id: Uuid::parse_str(&row.try_get::<String, _>("id")?)?,
        plugin_id: row.try_get("plugin_id")?,
        surface_id: row.try_get("surface_id")?,
        user_id: Uuid::parse_str(&row.try_get::<String, _>("user_id")?)?,
        entrypoint: row.try_get("entrypoint")?,
        package_digest: row.try_get("package_digest")?,
        channel_nonce: row.try_get("channel_nonce")?,
        allowed_bridge_methods: serde_json::from_str(
            &row.try_get::<String, _>("allowed_bridge_methods_json")?,
        )?,
        expires_at: row.try_get("expires_at")?,
    })
}
