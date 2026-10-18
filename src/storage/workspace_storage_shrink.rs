use serde::{Deserialize, Serialize};
use sqlx::Row;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{
    Database, StorageError,
    workspace_store::{decode_postgres, decode_sqlite, select_workspace_sql},
};
use crate::{
    templates::WorkspaceTemplateDocument,
    workspaces::{Workspace, WorkspaceHomeVolumeBinding, WorkspaceState},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StorageShrinkRequest {
    pub home_gib: Option<u64>,
    pub temporary_storage_gib: Option<u64>,
    pub expected_generation: u64,
    #[serde(default)]
    pub reclaim_source: bool,
}

impl StorageShrinkRequest {
    pub fn validate(&self, workspace: &Workspace) -> Result<(), StorageError> {
        let home = workspace
            .home_volume_binding
            .as_ref()
            .map_or(workspace.template.resources.disk_gib, |binding| {
                binding.capacity_gib
            });
        let temporary = workspace.template.storage_policy.temporary_storage_gib;
        if self.home_gib.is_none() && self.temporary_storage_gib.is_none()
            || self
                .home_gib
                .is_some_and(|target| target == 0 || target >= home)
            || self
                .temporary_storage_gib
                .is_some_and(|target| target == 0 || target >= temporary || target > 2048)
            || self.home_gib.is_some_and(|target| {
                target
                    .checked_mul(1 << 30)
                    .is_none_or(|bytes| bytes > i64::MAX as u64)
            })
        {
            return Err(StorageError::InvalidWorkspaceStorageShrink);
        }
        if workspace.generation != self.expected_generation
            || !matches!(
                workspace.state,
                WorkspaceState::Ready | WorkspaceState::Stopped
            )
        {
            return Err(StorageError::WorkspaceStorageShrinkConflict);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StorageShrinkPhase {
    Queued,
    Stopping,
    Measuring,
    Copying,
    Verifying,
    Switching,
    Resuming,
    RollingBack,
    Completed,
    Failed,
    Reclaiming,
    Reclaimed,
}

impl StorageShrinkPhase {
    pub fn active(self) -> bool {
        !matches!(self, Self::Completed | Self::Failed | Self::Reclaimed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StorageShrinkVolume {
    pub target_gib: Option<u64>,
    pub source: Option<WorkspaceHomeVolumeBinding>,
    pub destination: Option<WorkspaceHomeVolumeBinding>,
    pub used_bytes: Option<u64>,
    pub used_inodes: Option<u64>,
    pub verified: bool,
    pub source_volume: Option<StorageVolumeIdentity>,
    pub destination_volume: Option<StorageVolumeIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct StorageVolumeIdentity {
    pub pv_name: String,
    pub pv_uid: String,
    pub csi_driver: String,
    pub csi_volume_handle: String,
    pub storage_class_name: String,
    pub longhorn_namespace: Option<String>,
    pub longhorn_volume_uid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StorageShrinkOperation {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub phase: StorageShrinkPhase,
    pub error: Option<String>,
    pub home: StorageShrinkVolume,
    pub temporary: StorageShrinkVolume,
    pub resume: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub actor_user_id: Uuid,
    pub request: StorageShrinkRequest,
    pub original_home_gib: u64,
    pub original_temporary_storage_gib: u64,
    pub switched: bool,
    pub sources_retained: bool,
    pub reclaimable: bool,
    pub helper_pod_uids: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Copy)]
pub(crate) enum ShrinkWorkspaceUpdate {
    None,
    Stopping,
    Stopped,
    Switch,
    Resume,
    Rollback,
}

macro_rules! storage_backend {
    ($pool:expr, $installation:expr, $decode:ident, $suffix:expr, $body:expr) => {{
        let mut transaction = $pool.begin().await?;
        let select = format!("{}{}", select_workspace_sql("$1", "$2"), $suffix);
        let row = sqlx::query(&select)
            .bind($installation.as_str())
            .bind($body.workspace_id.to_string())
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(StorageError::WorkspaceNotFound)?;
        let yaml: String = row.try_get("template_snapshot_yaml")?;
        let workspace = $decode(row, $installation)?;
        (transaction, workspace, yaml)
    }};
}

impl Database {
    pub async fn list_storage_shrinks(
        &self,
        workspace_id: Uuid,
    ) -> Result<Vec<StorageShrinkOperation>, StorageError> {
        let query = "SELECT operation_json FROM workspace_storage_shrinks WHERE installation_id = $1 AND workspace_id = $2 ORDER BY created_at, id";
        let rows: Vec<String> = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query_scalar(query)
                    .bind(installation_id.as_str())
                    .bind(workspace_id.to_string())
                    .fetch_all(pool)
                    .await?
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query_scalar(query)
                    .bind(installation_id.as_str())
                    .bind(workspace_id.to_string())
                    .fetch_all(pool)
                    .await?
            }
        };
        rows.into_iter()
            .map(|json| serde_json::from_str(&json).map_err(StorageError::from))
            .collect()
    }

    pub(crate) async fn save_storage_shrink_cleanup_identity(
        &self,
        operation: &StorageShrinkOperation,
    ) -> Result<(), StorageError> {
        macro_rules! save_cleanup {
            ($pool:expr, $installation:expr, $decode:ident, $suffix:expr) => {{
                let (mut transaction, workspace, _) = storage_backend!($pool, $installation, $decode, $suffix, operation);
                if workspace.state != WorkspaceState::Deleting || operation.phase.active() {
                    return Err(StorageError::WorkspaceStorageShrinkConflict);
                }
                let json: String = sqlx::query_scalar("SELECT operation_json FROM workspace_storage_shrinks WHERE installation_id = $1 AND workspace_id = $2 AND id = $3 AND active = 0")
                    .bind($installation.as_str()).bind(workspace.id.to_string()).bind(operation.id.to_string())
                    .fetch_optional(&mut *transaction).await?.ok_or(StorageError::WorkspaceStorageShrinkConflict)?;
                let mut current: StorageShrinkOperation = serde_json::from_str(&json)?;
                for (role, existing, captured) in [("home", &mut current.home, &operation.home), ("temporary", &mut current.temporary, &operation.temporary)] {
                    if existing.source != captured.source || existing.destination.is_some() && existing.destination != captured.destination {
                        return Err(StorageError::WorkspaceStorageShrinkConflict);
                    }
                    if existing.destination.is_none()
                        && let Some(binding) = captured.destination.as_ref()
                    {
                        if !binding.is_valid() || binding.namespace != workspace.runtime.namespace()
                            || binding.claim_name != format!("shrink-{}-{role}", operation.id.simple())
                            || Some(binding.capacity_gib) != existing.target_gib
                        { return Err(StorageError::WorkspaceStorageShrinkConflict); }
                        existing.destination = Some(binding.clone());
                    }
                    if existing.source_volume.is_none() { existing.source_volume = captured.source_volume.clone(); }
                    if existing.destination_volume.is_none() { existing.destination_volume = captured.destination_volume.clone(); }
                }
                sqlx::query("UPDATE workspace_storage_shrinks SET operation_json = $1 WHERE installation_id = $2 AND workspace_id = $3 AND id = $4 AND active = 0")
                    .bind(serde_json::to_string(&current)?).bind($installation.as_str()).bind(workspace.id.to_string()).bind(operation.id.to_string())
                    .execute(&mut *transaction).await?;
                transaction.commit().await?;
                Ok(())
            }};
        }
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => save_cleanup!(pool, installation_id, decode_sqlite, ""),
            Self::Postgres {
                pool,
                installation_id,
            } => save_cleanup!(pool, installation_id, decode_postgres, " FOR UPDATE"),
        }
    }

    pub async fn confirm_storage_shrink(
        &self,
        workspace_id: Uuid,
        operation_id: Uuid,
        actor_user_id: Uuid,
        now: i64,
    ) -> Result<StorageShrinkOperation, StorageError> {
        let context = StorageShrinkOperation::new(
            workspace_id,
            StorageShrinkRequest {
                home_gib: None,
                temporary_storage_gib: None,
                expected_generation: 0,
                reclaim_source: false,
            },
            actor_user_id,
            now,
        );
        macro_rules! confirm {
            ($pool:expr, $installation:expr, $decode:ident, $suffix:expr) => {{
                let (mut transaction, workspace, _) = storage_backend!($pool, $installation, $decode, $suffix, context);
                let json: String = sqlx::query_scalar("SELECT operation_json FROM workspace_storage_shrinks WHERE installation_id = $1 AND workspace_id = $2 ORDER BY created_at DESC, id DESC LIMIT 1")
                    .bind($installation.as_str()).bind(workspace_id.to_string()).fetch_optional(&mut *transaction).await?.ok_or(StorageError::WorkspaceStorageShrinkConflict)?;
                let mut operation: StorageShrinkOperation = serde_json::from_str(&json)?;
                if operation.id != operation_id { return Err(StorageError::WorkspaceStorageShrinkConflict); }
                if matches!(operation.phase, StorageShrinkPhase::Reclaiming | StorageShrinkPhase::Reclaimed) { return Ok(operation); }
                if operation.phase != StorageShrinkPhase::Completed || !operation.reclaimable
                    || operation.resume && workspace.state != WorkspaceState::Ready
                    || !operation.resume && workspace.state != WorkspaceState::Stopped
                { return Err(StorageError::WorkspaceStorageShrinkConflict); }
                let active: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workspace_storage_shrinks WHERE installation_id = $1 AND workspace_id = $2 AND active = 1")
                    .bind($installation.as_str()).bind(workspace_id.to_string()).fetch_one(&mut *transaction).await?;
                if active != 0 { return Err(StorageError::WorkspaceStorageShrinkConflict); }
                operation.phase = StorageShrinkPhase::Reclaiming;
                operation.updated_at = now;
                operation.error = None;
                sqlx::query("UPDATE workspace_storage_shrinks SET active = 1, operation_json = $1, updated_at = $2 WHERE installation_id = $3 AND id = $4")
                    .bind(serde_json::to_string(&operation)?).bind(now).bind($installation.as_str()).bind(operation.id.to_string()).execute(&mut *transaction).await?;
                sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES ($1,$2,'workspace_storage_shrink',$3,$4,'pending',$5,NULL,NULL,0,$5,$5)")
                    .bind(Uuid::now_v7().to_string()).bind($installation.as_str()).bind(workspace_id.to_string()).bind(serde_json::json!({"operation_id": operation.id}).to_string()).bind(now).execute(&mut *transaction).await?;
                sqlx::query("UPDATE workspaces SET generation = generation + 1, updated_at = $1 WHERE installation_id = $2 AND id = $3")
                    .bind(now).bind($installation.as_str()).bind(workspace_id.to_string()).execute(&mut *transaction).await?;
                sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES ($1,$2,$3,$4,$5,'workspace.storage_shrink_confirmed',$6,$7)")
                    .bind(Uuid::now_v7().to_string()).bind($installation.as_str()).bind(actor_user_id.to_string()).bind(workspace.organization_id.to_string()).bind(workspace_id.to_string()).bind(serde_json::json!({"operation_id": operation.id}).to_string()).bind(now).execute(&mut *transaction).await?;
                transaction.commit().await?;
                Ok(operation)
            }};
        }
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => confirm!(pool, installation_id, decode_sqlite, ""),
            Self::Postgres {
                pool,
                installation_id,
            } => confirm!(pool, installation_id, decode_postgres, " FOR UPDATE"),
        }
    }

    pub async fn retry_storage_shrink_resume(
        &self,
        workspace_id: Uuid,
        operation_id: Uuid,
        actor_user_id: Uuid,
        now: i64,
    ) -> Result<StorageShrinkOperation, StorageError> {
        macro_rules! retry {
            ($pool:expr, $installation:expr, $decode:ident, $suffix:expr) => {{
                let mut transaction = $pool.begin().await?;
                if $suffix.is_empty() {
                    sqlx::query("UPDATE workspaces SET generation = generation WHERE installation_id = $1 AND id = $2")
                        .bind($installation.as_str()).bind(workspace_id.to_string()).execute(&mut *transaction).await?;
                }
                let select = format!("{}{}", select_workspace_sql("$1", "$2"), $suffix);
                let row = sqlx::query(&select)
                    .bind($installation.as_str()).bind(workspace_id.to_string())
                    .fetch_optional(&mut *transaction).await?.ok_or(StorageError::WorkspaceNotFound)?;
                let workspace = $decode(row, $installation)?;
                let row = sqlx::query("SELECT id, operation_json FROM workspace_storage_shrinks WHERE installation_id = $1 AND workspace_id = $2 ORDER BY created_at DESC, id DESC LIMIT 1")
                    .bind($installation.as_str()).bind(workspace_id.to_string())
                    .fetch_optional(&mut *transaction).await?.ok_or(StorageError::WorkspaceStorageShrinkConflict)?;
                let stored_id: String = row.try_get("id")?;
                let json: String = row.try_get("operation_json")?;
                let mut operation: StorageShrinkOperation = serde_json::from_str(&json)?;
                validate_retry_operation(&stored_id, operation_id, &operation, &workspace)?;
                validate_retry_volume(&operation.home, operation.request.home_gib, workspace.home_volume_binding.as_ref(), workspace.template.resources.disk_gib, operation.original_home_gib, workspace.runtime.namespace())?;
                validate_retry_volume(&operation.temporary, operation.request.temporary_storage_gib, workspace.scratch_volume_binding.as_ref(), workspace.template.storage_policy.temporary_storage_gib, operation.original_temporary_storage_gib, workspace.runtime.namespace())?;
                let active: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workspace_storage_shrinks WHERE installation_id = $1 AND workspace_id = $2 AND active = 1")
                    .bind($installation.as_str()).bind(workspace_id.to_string()).fetch_one(&mut *transaction).await?;
                let jobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE installation_id = $1 AND workspace_id = $2 AND status IN ('pending', 'running')")
                    .bind($installation.as_str()).bind(workspace_id.to_string()).fetch_one(&mut *transaction).await?;
                if active != 0 || jobs != 0 { return Err(StorageError::WorkspaceStorageShrinkConflict); }
                let prior_error = operation.error.take();
                operation.phase = StorageShrinkPhase::Resuming;
                operation.updated_at = now;
                let updated = sqlx::query("UPDATE workspace_storage_shrinks SET active = 1, operation_json = $1, updated_at = $2 WHERE installation_id = $3 AND workspace_id = $4 AND id = $5 AND active = 0")
                    .bind(serde_json::to_string(&operation)?).bind(now).bind($installation.as_str()).bind(workspace_id.to_string()).bind(operation.id.to_string()).execute(&mut *transaction).await?;
                if updated.rows_affected() != 1 { return Err(StorageError::WorkspaceStorageShrinkConflict); }
                sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES ($1,$2,'workspace_storage_shrink',$3,$4,'pending',$5,NULL,NULL,0,$5,$5)")
                    .bind(Uuid::now_v7().to_string()).bind($installation.as_str()).bind(workspace_id.to_string()).bind(serde_json::json!({"operation_id": operation.id}).to_string()).bind(now).execute(&mut *transaction).await?;
                sqlx::query("UPDATE workspaces SET generation = generation + 1, updated_at = $1 WHERE installation_id = $2 AND id = $3")
                    .bind(now).bind($installation.as_str()).bind(workspace_id.to_string()).execute(&mut *transaction).await?;
                sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES ($1,$2,$3,$4,$5,'workspace.storage_shrink_resume_retried',$6,$7)")
                    .bind(Uuid::now_v7().to_string()).bind($installation.as_str()).bind(actor_user_id.to_string()).bind(workspace.organization_id.to_string()).bind(workspace_id.to_string()).bind(serde_json::json!({"operation_id": operation.id, "prior_error": prior_error}).to_string()).bind(now).execute(&mut *transaction).await?;
                transaction.commit().await?;
                Ok(operation)
            }};
        }
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => retry!(pool, installation_id, decode_sqlite, ""),
            Self::Postgres {
                pool,
                installation_id,
            } => retry!(pool, installation_id, decode_postgres, " FOR UPDATE"),
        }
    }

    pub async fn latest_storage_shrink(
        &self,
        workspace_id: Uuid,
    ) -> Result<Option<StorageShrinkOperation>, StorageError> {
        let query = "SELECT operation_json FROM workspace_storage_shrinks WHERE installation_id = $1 AND workspace_id = $2 ORDER BY created_at DESC, id DESC LIMIT 1";
        let json: Option<String> = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query_scalar(query)
                    .bind(installation_id.as_str())
                    .bind(workspace_id.to_string())
                    .fetch_optional(pool)
                    .await?
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query_scalar(query)
                    .bind(installation_id.as_str())
                    .bind(workspace_id.to_string())
                    .fetch_optional(pool)
                    .await?
            }
        };
        json.map(|value| serde_json::from_str(&value).map_err(StorageError::from))
            .transpose()
    }

    pub async fn storage_shrink_active(&self, workspace_id: Uuid) -> Result<bool, StorageError> {
        let query = "SELECT COUNT(*) FROM workspace_storage_shrinks WHERE installation_id = $1 AND workspace_id = $2 AND active = 1";
        let count: i64 = match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => {
                sqlx::query_scalar(query)
                    .bind(installation_id.as_str())
                    .bind(workspace_id.to_string())
                    .fetch_one(pool)
                    .await?
            }
            Self::Postgres {
                pool,
                installation_id,
            } => {
                sqlx::query_scalar(query)
                    .bind(installation_id.as_str())
                    .bind(workspace_id.to_string())
                    .fetch_one(pool)
                    .await?
            }
        };
        Ok(count != 0)
    }

    pub async fn ensure_storage_not_shrinking(
        &self,
        workspace_id: Uuid,
    ) -> Result<(), StorageError> {
        if self.storage_shrink_active(workspace_id).await? {
            return Err(StorageError::WorkspaceStorageShrinkConflict);
        }
        Ok(())
    }

    pub async fn enqueue_storage_shrink(
        &self,
        workspace_id: Uuid,
        request: StorageShrinkRequest,
        actor_user_id: Uuid,
        now: i64,
    ) -> Result<StorageShrinkOperation, StorageError> {
        let placeholder = StorageShrinkOperation::new(workspace_id, request, actor_user_id, now);
        macro_rules! enqueue {
            ($pool:expr, $installation:expr, $decode:ident, $suffix:expr) => {{
                let (mut transaction, workspace, _) = storage_backend!($pool, $installation, $decode, $suffix, placeholder);
                let active: Option<String> = sqlx::query_scalar("SELECT operation_json FROM workspace_storage_shrinks WHERE installation_id = $1 AND workspace_id = $2 AND active = 1")
                    .bind($installation.as_str()).bind(workspace_id.to_string()).fetch_optional(&mut *transaction).await?;
                if let Some(json) = active {
                    let operation: StorageShrinkOperation = serde_json::from_str(&json)?;
                    if operation.request == placeholder.request && operation.actor_user_id == actor_user_id { return Ok(operation); }
                    return Err(StorageError::WorkspaceStorageShrinkConflict);
                }
                let previous: Option<String> = sqlx::query_scalar("SELECT operation_json FROM workspace_storage_shrinks WHERE installation_id = $1 AND workspace_id = $2 ORDER BY created_at DESC, id DESC LIMIT 1")
                    .bind($installation.as_str()).bind(workspace_id.to_string()).fetch_optional(&mut *transaction).await?;
                if let Some(json) = previous {
                    let previous: StorageShrinkOperation = serde_json::from_str(&json)?;
                    if previous.reclaimable { return Err(StorageError::WorkspaceStorageShrinkConflict); }
                }
                placeholder.request.validate(&workspace)?;
                let mut operation = placeholder;
                operation.resume = workspace.state == WorkspaceState::Ready;
                operation.original_home_gib = workspace.template.resources.disk_gib;
                operation.original_temporary_storage_gib = workspace.template.storage_policy.temporary_storage_gib;
                operation.home.source = workspace.home_volume_binding;
                operation.temporary.source = workspace.scratch_volume_binding;
                sqlx::query("INSERT INTO workspace_storage_shrinks (id, installation_id, workspace_id, operation_json, active, created_at, updated_at) VALUES ($1,$2,$3,$4,1,$5,$5)")
                    .bind(operation.id.to_string()).bind($installation.as_str()).bind(workspace_id.to_string()).bind(serde_json::to_string(&operation)?).bind(now).execute(&mut *transaction).await?;
                sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES ($1,$2,'workspace_storage_shrink',$3,$4,'pending',$5,NULL,NULL,0,$5,$5)")
                    .bind(Uuid::now_v7().to_string()).bind($installation.as_str()).bind(workspace_id.to_string()).bind(serde_json::json!({"operation_id": operation.id}).to_string()).bind(now).execute(&mut *transaction).await?;
                sqlx::query("UPDATE workspaces SET generation = generation + 1, updated_at = $1 WHERE installation_id = $2 AND id = $3")
                    .bind(now).bind($installation.as_str()).bind(workspace_id.to_string()).execute(&mut *transaction).await?;
                sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES ($1,$2,$3,$4,$5,'workspace.storage_shrink_requested',$6,$7)")
                    .bind(Uuid::now_v7().to_string()).bind($installation.as_str()).bind(actor_user_id.to_string()).bind(workspace.organization_id.to_string()).bind(workspace_id.to_string()).bind(serde_json::to_string(&operation.request)?).bind(now).execute(&mut *transaction).await?;
                transaction.commit().await?;
                Ok(operation)
            }};
        }
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => enqueue!(pool, installation_id, decode_sqlite, ""),
            Self::Postgres {
                pool,
                installation_id,
            } => enqueue!(pool, installation_id, decode_postgres, " FOR UPDATE"),
        }
    }

    pub(crate) async fn save_storage_shrink(
        &self,
        operation: &StorageShrinkOperation,
        update: ShrinkWorkspaceUpdate,
    ) -> Result<(), StorageError> {
        macro_rules! save {
            ($pool:expr, $installation:expr, $decode:ident, $suffix:expr) => {{
                let (mut transaction, mut workspace, yaml) = storage_backend!($pool, $installation, $decode, $suffix, operation);
                let previous: Option<String> = sqlx::query_scalar("SELECT operation_json FROM workspace_storage_shrinks WHERE installation_id = $1 AND id = $2 AND workspace_id = $3 AND active = 1")
                    .bind($installation.as_str()).bind(operation.id.to_string()).bind(workspace.id.to_string()).fetch_optional(&mut *transaction).await?;
                if previous.is_none() { return Err(StorageError::WorkspaceStorageShrinkConflict); }
                if !matches!(update, ShrinkWorkspaceUpdate::None) {
                    let mut document: WorkspaceTemplateDocument = serde_yaml_ng::from_str(&yaml).map_err(|_| StorageError::InvalidWorkspace)?;
                    apply_shrink_workspace_update(&mut workspace, &mut document, operation, update)?;
                    let home = workspace.home_volume_binding.as_ref();
                    sqlx::query("UPDATE workspaces SET state = $1, home_pvc_namespace = $2, home_pvc_name = $3, home_pvc_uid = $4, home_pvc_capacity_gib = $5, scratch_volume_binding_json = $6, disk_gib = $7, temporary_storage_gib = $8, template_snapshot_yaml = $9, generation = generation + 1, updated_at = $10 WHERE installation_id = $11 AND id = $12")
                        .bind(workspace.state.as_str()).bind(home.map(|binding| binding.namespace.as_str())).bind(home.map(|binding| binding.claim_name.as_str())).bind(home.map(|binding| binding.claim_uid.as_str())).bind(home.map(|binding| binding.capacity_gib as i64))
                        .bind(workspace.scratch_volume_binding.as_ref().map(serde_json::to_string).transpose()?).bind(document.spec.resources.disk_gib as i64).bind(document.spec.storage_policy.temporary_storage_gib as i64)
                        .bind(serde_yaml_ng::to_string(&document).map_err(|_| StorageError::InvalidWorkspace)?).bind(operation.updated_at).bind($installation.as_str()).bind(workspace.id.to_string()).execute(&mut *transaction).await?;
                }
                sqlx::query("UPDATE workspace_storage_shrinks SET operation_json = $1, active = $2, updated_at = $3 WHERE installation_id = $4 AND id = $5")
                    .bind(serde_json::to_string(operation)?).bind(i64::from(operation.phase.active())).bind(operation.updated_at).bind($installation.as_str()).bind(operation.id.to_string()).execute(&mut *transaction).await?;
                if !operation.phase.active() {
                    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
                        .bind(Uuid::now_v7().to_string()).bind($installation.as_str()).bind(operation.actor_user_id.to_string()).bind(workspace.organization_id.to_string()).bind(workspace.id.to_string())
                        .bind(match operation.phase { StorageShrinkPhase::Completed => "workspace.storage_shrink_completed", StorageShrinkPhase::Reclaimed => "workspace.storage_shrink_reclaimed", _ => "workspace.storage_shrink_failed" }).bind(serde_json::to_string(operation)?).bind(operation.updated_at).execute(&mut *transaction).await?;
                }
                transaction.commit().await?;
                Ok(())
            }};
        }
        match self {
            Self::Sqlite {
                pool,
                installation_id,
            } => save!(pool, installation_id, decode_sqlite, ""),
            Self::Postgres {
                pool,
                installation_id,
            } => save!(pool, installation_id, decode_postgres, " FOR UPDATE"),
        }
    }
}

impl StorageShrinkOperation {
    pub(crate) fn new(
        workspace_id: Uuid,
        request: StorageShrinkRequest,
        actor_user_id: Uuid,
        now: i64,
    ) -> Self {
        let volume = |target_gib| StorageShrinkVolume {
            target_gib,
            source: None,
            destination: None,
            used_bytes: None,
            used_inodes: None,
            verified: false,
            source_volume: None,
            destination_volume: None,
        };
        Self {
            id: Uuid::now_v7(),
            workspace_id,
            phase: StorageShrinkPhase::Queued,
            error: None,
            home: volume(request.home_gib),
            temporary: volume(request.temporary_storage_gib),
            resume: false,
            created_at: now,
            updated_at: now,
            actor_user_id,
            request,
            original_home_gib: 0,
            original_temporary_storage_gib: 0,
            switched: false,
            sources_retained: true,
            reclaimable: false,
            helper_pod_uids: Default::default(),
        }
    }
}

fn apply_shrink_workspace_update(
    workspace: &mut Workspace,
    document: &mut WorkspaceTemplateDocument,
    operation: &StorageShrinkOperation,
    update: ShrinkWorkspaceUpdate,
) -> Result<(), StorageError> {
    match update {
        ShrinkWorkspaceUpdate::Stopping => {
            workspace.state = WorkspaceState::Stopping;
            if !operation.switched {
                workspace.scratch_volume_binding = operation.temporary.source.clone();
            }
        }
        ShrinkWorkspaceUpdate::Stopped => workspace.state = WorkspaceState::Stopped,
        ShrinkWorkspaceUpdate::Switch | ShrinkWorkspaceUpdate::Rollback => {
            let switching = matches!(update, ShrinkWorkspaceUpdate::Switch);
            if switching
                && (operation.home.target_gib.is_some() && !operation.home.verified
                    || operation.temporary.target_gib.is_some() && !operation.temporary.verified)
            {
                return Err(StorageError::InvalidWorkspaceStorageShrink);
            }
            if switching {
                validate_verified_destination(&operation.home)?;
                validate_verified_destination(&operation.temporary)?;
            }
            workspace.home_volume_binding = if switching {
                operation
                    .home
                    .destination
                    .clone()
                    .or(operation.home.source.clone())
            } else {
                operation.home.source.clone()
            };
            workspace.scratch_volume_binding = if switching {
                operation
                    .temporary
                    .destination
                    .clone()
                    .or(operation.temporary.source.clone())
            } else {
                operation.temporary.source.clone()
            };
            document.spec.resources.disk_gib = if switching {
                operation
                    .home
                    .target_gib
                    .unwrap_or(operation.original_home_gib)
            } else {
                operation.original_home_gib
            };
            document.spec.storage_policy.temporary_storage_gib = if switching {
                operation
                    .temporary
                    .target_gib
                    .unwrap_or(operation.original_temporary_storage_gib)
            } else {
                operation.original_temporary_storage_gib
            };
            workspace.state = WorkspaceState::Stopped;
        }
        ShrinkWorkspaceUpdate::Resume => workspace.state = WorkspaceState::Starting,
        ShrinkWorkspaceUpdate::None => unreachable!(),
    }
    Ok(())
}

fn validate_retry_operation(
    stored_id: &str,
    operation_id: Uuid,
    operation: &StorageShrinkOperation,
    workspace: &Workspace,
) -> Result<(), StorageError> {
    if stored_id != operation_id.to_string()
        || operation.id != operation_id
        || operation.workspace_id != workspace.id
        || operation.phase != StorageShrinkPhase::Failed
        || !operation.switched
        || !operation.sources_retained
        || operation.reclaimable
        || operation.request.reclaim_source
        || !operation.resume
        || workspace.state != WorkspaceState::Ready
        || operation.request.home_gib.is_none() && operation.request.temporary_storage_gib.is_none()
    {
        return Err(StorageError::WorkspaceStorageShrinkConflict);
    }
    Ok(())
}

fn validate_retry_volume(
    volume: &StorageShrinkVolume,
    requested_target: Option<u64>,
    current: Option<&WorkspaceHomeVolumeBinding>,
    current_gib: u64,
    original_gib: u64,
    namespace: &str,
) -> Result<(), StorageError> {
    if volume.target_gib != requested_target {
        return Err(StorageError::WorkspaceStorageShrinkConflict);
    }
    let Some(target) = requested_target else {
        if volume.source.as_ref() != current
            || volume.destination.is_some()
            || current_gib != original_gib
        {
            return Err(StorageError::WorkspaceStorageShrinkConflict);
        }
        return Ok(());
    };
    validate_verified_destination(volume)
        .map_err(|_| StorageError::WorkspaceStorageShrinkConflict)?;
    let identity_recorded = |identity: &StorageVolumeIdentity| {
        [
            &identity.pv_name,
            &identity.pv_uid,
            &identity.csi_driver,
            &identity.csi_volume_handle,
            &identity.storage_class_name,
        ]
        .iter()
        .all(|value| !value.trim().is_empty())
    };
    let valid = volume.destination.as_ref() == current
        && current_gib == target
        && volume.source.as_ref().is_some_and(|source| {
            source.namespace == namespace
                && source.capacity_gib == original_gib
                && volume.destination.as_ref().is_some_and(|destination| {
                    destination.namespace == namespace
                        && source.claim_name != destination.claim_name
                })
        })
        && volume.source_volume.as_ref().is_some_and(|source| {
            identity_recorded(source)
                && volume
                    .destination_volume
                    .as_ref()
                    .is_some_and(|destination| {
                        identity_recorded(destination) && source.pv_name != destination.pv_name
                    })
        });
    if !valid {
        return Err(StorageError::WorkspaceStorageShrinkConflict);
    }
    Ok(())
}

fn validate_verified_destination(volume: &StorageShrinkVolume) -> Result<(), StorageError> {
    let Some(target) = volume.target_gib else {
        return Ok(());
    };
    let valid = volume.verified
        && volume
            .source
            .as_ref()
            .is_some_and(|source| source.is_valid() && source.capacity_gib > target)
        && volume.destination.as_ref().is_some_and(|destination| {
            destination.is_valid()
                && destination.capacity_gib == target
                && volume
                    .source
                    .as_ref()
                    .is_some_and(|source| source.claim_uid != destination.claim_uid)
        })
        && target
            .checked_mul(1 << 30)
            .is_some_and(|bytes| volume.used_bytes.is_some_and(|used| bytes > used))
        && volume.used_inodes.is_some()
        && volume
            .destination_volume
            .as_ref()
            .is_some_and(|destination| {
                volume.source_volume.as_ref().is_some_and(|source| {
                    source.pv_uid != destination.pv_uid
                        && source.csi_driver == destination.csi_driver
                        && source.storage_class_name == destination.storage_class_name
                        && source.csi_volume_handle != destination.csi_volume_handle
                })
            });
    if !valid {
        return Err(StorageError::InvalidWorkspaceStorageShrink);
    }
    Ok(())
}

pub(super) async fn guard_sqlite(
    connection: &mut sqlx::SqliteConnection,
    installation: &str,
    workspace_id: Uuid,
) -> Result<(), StorageError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workspace_storage_shrinks WHERE installation_id = $1 AND workspace_id = $2 AND active = 1")
        .bind(installation).bind(workspace_id.to_string()).fetch_one(connection).await?;
    if count != 0 {
        return Err(StorageError::WorkspaceStorageShrinkConflict);
    }
    Ok(())
}

pub(super) async fn guard_postgres(
    connection: &mut sqlx::PgConnection,
    installation: &str,
    workspace_id: Uuid,
) -> Result<(), StorageError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workspace_storage_shrinks WHERE installation_id = $1 AND workspace_id = $2 AND active = 1")
        .bind(installation).bind(workspace_id.to_string()).fetch_one(connection).await?;
    if count != 0 {
        return Err(StorageError::WorkspaceStorageShrinkConflict);
    }
    Ok(())
}

#[cfg(test)]
#[path = "workspace_storage_shrink/tests.rs"]
mod tests;
