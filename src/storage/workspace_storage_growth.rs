use serde::{Deserialize, Serialize};
use sqlx::Row;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{
    Database, StorageError,
    workspace_settings::{admit_postgres, admit_sqlite},
    workspace_store::{decode_postgres, decode_sqlite, select_workspace_sql},
};
use crate::{
    kubernetes::storage_growth::{GrowthClaim, GrowthVolume},
    templates::WorkspaceTemplateDocument,
    workspaces::{Workspace, WorkspaceState},
};

pub(crate) const STORAGE_GROWTH_JOB: &str = "workspace_storage_growth";

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct StorageGrowth {
    pub volume: GrowthVolume,
    pub claim: GrowthClaim,
    pub previous_gib: u64,
    pub requested_gib: u64,
    pub rejection: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct StorageGrowthStatus {
    pub id: Uuid,
    pub volume: GrowthVolume,
    pub previous_gib: u64,
    pub requested_gib: u64,
    pub status: String,
    pub rejection: Option<String>,
}

const ACTIVE: &str = "SELECT COUNT(*) FROM jobs WHERE installation_id = $1 +    AND workspace_id = $2 AND kind = 'workspace_storage_growth' AND status IN ('pending', 'running')";

macro_rules! growth_guard {
    ($name:ident, $connection:ty) => {
        pub(super) async fn $name(
            connection: &mut $connection,
            installation: &str,
            id: Uuid,
        ) -> Result<(), StorageError> {
            let count: i64 = sqlx::query_scalar(ACTIVE)
                .bind(installation)
                .bind(id.to_string())
                .fetch_one(connection)
                .await?;
            if count != 0 {
                return Err(StorageError::WorkspaceStorageGrowthConflict);
            }
            Ok(())
        }
    };
}

growth_guard!(guard_sqlite, sqlx::SqliteConnection);
growth_guard!(guard_postgres, sqlx::PgConnection);

macro_rules! locked_workspace {
    ($pool:expr, $installation:expr, $id:expr, $decode:ident, $lock:expr) => {{
        let mut tx = $pool.begin().await?;
        let row = sqlx::query(&format!("{}{}", select_workspace_sql("$1", "$2"), $lock))
            .bind($installation.as_str())
            .bind($id.to_string())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(StorageError::WorkspaceNotFound)?;
        let yaml: String = row.try_get("template_snapshot_yaml")?;
        (tx, $decode(row, $installation)?, yaml)
    }};
}

impl Database {
    pub(crate) async fn enqueue_storage_growth(
        &self,
        id: Uuid,
        generation: u64,
        actor: Uuid,
        growth: StorageGrowth,
        now: i64,
    ) -> Result<StorageGrowthStatus, StorageError> {
        let job = Uuid::now_v7();
        let payload = serde_json::to_string(&growth)?;
        macro_rules! enqueue {
            ($pool:expr, $installation:expr, $decode:ident, $lock:expr, $admit:ident, $guard:ident) => {{
                let (mut tx, mut workspace, _) =
                    locked_workspace!($pool, $installation, id, $decode, $lock);
                growth.check_current(&workspace, generation)?;
                super::workspace_storage_shrink::$guard(&mut tx, $installation.as_str(), id)
                    .await?;
                $guard(&mut tx, $installation.as_str(), id).await?;
                // Charge the requested Home size before CSI can allocate it. The snapshot
                // and scratch size continue to describe the filesystem actually in use.
                if matches!(growth.volume, GrowthVolume::Home) {
                    workspace.template.resources.disk_gib = growth.requested_gib;
                    $admit(&mut tx, $installation.as_str(), &workspace).await?;
                    sqlx::query("UPDATE workspaces SET disk_gib = $1 WHERE installation_id = $2 AND id = $3")
                        .bind(growth.requested_gib as i64).bind($installation.as_str())
                        .bind(id.to_string()).execute(&mut *tx).await?;
                }
                sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, attempts, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, 'pending', $6, 0, $6, $6)")
                    .bind(job.to_string()).bind($installation.as_str()).bind(STORAGE_GROWTH_JOB)
                    .bind(id.to_string()).bind(&payload).bind(now).execute(&mut *tx).await?;
                sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES ($1, $2, $3, $4, $5, 'workspace.storage_growth_requested', $6, $7)")
                    .bind(Uuid::now_v7().to_string()).bind($installation.as_str())
                    .bind(actor.to_string()).bind(workspace.organization_id.to_string())
                    .bind(id.to_string()).bind(&payload).bind(now).execute(&mut *tx).await?;
                tx.commit().await?;
            }};
        }
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                enqueue!(
                    pool,
                    installation_id,
                    decode_sqlite,
                    "",
                    admit_sqlite,
                    guard_sqlite
                )
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                enqueue!(
                    pool,
                    installation_id,
                    decode_postgres,
                    " FOR UPDATE",
                    admit_postgres,
                    guard_postgres
                )
            }
        }
        Ok(growth.status(job, "pending".to_owned()))
    }

    pub(crate) async fn latest_storage_growth(
        &self,
        workspace: Uuid,
    ) -> Result<Option<StorageGrowthStatus>, StorageError> {
        const QUERY: &str = "SELECT id, payload_json, status FROM jobs WHERE installation_id = $1 +            AND workspace_id = $2 AND kind = 'workspace_storage_growth' ORDER BY id DESC LIMIT 1";
        let workspace = workspace.to_string();
        let row: Option<(String, String, String)> = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query_as(QUERY)
                    .bind(installation_id.as_str())
                    .bind(&workspace)
                    .fetch_optional(pool)
                    .await?
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query_as(QUERY)
                    .bind(installation_id.as_str())
                    .bind(&workspace)
                    .fetch_optional(pool)
                    .await?
            }
        };
        row.map(|(id, payload, status)| {
            let id = Uuid::parse_str(&id).map_err(|_| StorageError::InvalidWorkspace)?;
            let growth: StorageGrowth = serde_json::from_str(&payload)?;
            Ok(growth.status(id, status))
        })
        .transpose()
    }

    pub(crate) async fn apply_storage_growth(
        &self,
        id: Uuid,
        growth: &StorageGrowth,
        now: i64,
    ) -> Result<(), StorageError> {
        macro_rules! apply {
            ($pool:expr, $installation:expr, $decode:ident, $lock:expr, $event:ident) => {{
                let (mut tx, mut workspace, yaml) =
                    locked_workspace!($pool, $installation, id, $decode, $lock);
                let current = growth.volume.capacity(&workspace);
                if current == growth.requested_gib {
                    return Ok(());
                }
                if current != growth.previous_gib {
                    return Err(StorageError::WorkspaceStorageGrowthConflict);
                }
                let mut snapshot = WorkspaceTemplateDocument::parse(&yaml)
                    .map_err(|_| StorageError::InvalidWorkspace)?;
                let binding = match growth.volume {
                    GrowthVolume::Home => {
                        snapshot.spec.resources.disk_gib = growth.requested_gib;
                        workspace.home_volume_binding.as_mut()
                    }
                    GrowthVolume::TemporaryStorage => {
                        snapshot.spec.storage_policy.temporary_storage_gib = growth.requested_gib;
                        workspace.scratch_volume_binding.as_mut()
                    }
                };
                if let Some(binding) = binding {
                    if binding.claim_uid != growth.claim.uid || binding.claim_name != growth.claim.name {
                        return Err(StorageError::WorkspaceStorageGrowthConflict);
                    }
                    binding.capacity_gib = growth.requested_gib;
                }
                let yaml = snapshot.to_yaml().map_err(|_| StorageError::InvalidWorkspace)?;
                workspace.template = snapshot.spec;
                workspace.updated_at = now;
                sqlx::query("UPDATE workspaces SET template_snapshot_yaml = $1, disk_gib = $2, temporary_storage_gib = $3, home_pvc_capacity_gib = $4, scratch_volume_binding_json = $5, updated_at = $6 WHERE installation_id = $7 AND id = $8")
                    .bind(yaml)
                    .bind(workspace.template.resources.disk_gib as i64)
                    .bind(workspace.template.storage_policy.temporary_storage_gib as i64)
                    .bind(workspace.home_volume_binding.as_ref().map(|b| b.capacity_gib as i64))
                    .bind(workspace.scratch_volume_binding.as_ref().map(serde_json::to_string).transpose()?)
                    .bind(now).bind($installation.as_str()).bind(id.to_string())
                    .execute(&mut *tx).await?;
                // Capacity does not change the workload revision.
                super::workspace_events::$event(
                    &mut tx, $installation.as_str(), &workspace, Some("storage_grown"), now,
                ).await?;
                tx.commit().await?;
            }};
        }
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => apply!(pool, installation_id, decode_sqlite, "", insert_sqlite),
            Self::Postgres {
                pool,
                installation_id,
            } => apply!(
                pool,
                installation_id,
                decode_postgres,
                " FOR UPDATE",
                insert_postgres
            ),
        }
        Ok(())
    }

    pub(crate) async fn reject_storage_growth(
        &self,
        id: Uuid,
        job: Uuid,
        growth: &mut StorageGrowth,
        reason: &str,
    ) -> Result<(), StorageError> {
        growth.rejection = Some(reason.to_owned());
        let payload = serde_json::to_string(&growth)?;
        // Persist the refusal and release its reservation together. A worker
        // that retries the job after a crash must not reapply the PVC patch.
        macro_rules! reject {
            ($pool:expr, $installation:expr) => {{
                let mut tx = $pool.begin().await?;
                if matches!(growth.volume, GrowthVolume::Home) {
                    sqlx::query("UPDATE workspaces SET disk_gib = $1 WHERE installation_id = $2 AND id = $3 AND disk_gib = $4")
                        .bind(growth.previous_gib as i64).bind($installation.as_str())
                        .bind(id.to_string()).bind(growth.requested_gib as i64).execute(&mut *tx).await?;
                }
                sqlx::query("UPDATE jobs SET payload_json = $1 WHERE installation_id = $2 AND id = $3 AND status = 'running'")
                    .bind(&payload).bind($installation.as_str()).bind(job.to_string())
                    .execute(&mut *tx).await?;
                tx.commit().await?;
            }};
        }
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => reject!(pool, installation_id),
            Self::Postgres {
                pool,
                installation_id,
            } => reject!(pool, installation_id),
        }
        Ok(())
    }
}

impl StorageGrowth {
    fn check_current(&self, workspace: &Workspace, generation: u64) -> Result<(), StorageError> {
        if workspace.state != WorkspaceState::Ready
            || workspace.generation != generation
            || self.volume.capacity(workspace) != self.previous_gib
        {
            return Err(StorageError::WorkspaceStorageGrowthConflict);
        }
        Ok(())
    }

    fn status(&self, id: Uuid, status: String) -> StorageGrowthStatus {
        StorageGrowthStatus {
            id,
            volume: self.volume,
            previous_gib: self.previous_gib,
            requested_gib: self.requested_gib,
            status,
            rejection: self.rejection.clone(),
        }
    }
}
