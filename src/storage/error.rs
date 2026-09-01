use thiserror::Error;
use uuid::Uuid;

use crate::config::{ConfigError, InstallationId};

#[derive(Debug, Error)]
pub enum StorageError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Uuid(#[from] uuid::Error),
    #[error("lease owner must not be empty")]
    EmptyLeaseOwner,
    #[error("lease duration overflowed")]
    LeaseDurationOverflow,
    #[error("job {0} is not leased by this owner")]
    LeaseNotOwned(Uuid),
    #[error("database belongs to installation {stored}, not configured installation {configured}")]
    InstallationMismatch {
        configured: InstallationId,
        stored: String,
    },
    #[error("API tokens must contain at least 32 bytes")]
    TokenTooShort,
    #[error("authenticated user was not found")]
    UserNotFound,
    #[error("display name or avatar upload is invalid")]
    InvalidUserProfile,
    #[error("API key name is invalid")]
    InvalidApiKey,
    #[error("API key was not found")]
    ApiKeyNotFound,
    #[error("the last active API key cannot be revoked")]
    LastApiKey,
    #[error("a user may have at most 20 active API keys")]
    TooManyApiKeys,
    #[error("audit pagination or filters are invalid")]
    InvalidAuditQuery,
    #[error("organization was not found")]
    OrganizationNotFound,
    #[error("the last active system administrator cannot be disabled or demoted")]
    LastSystemAdmin,
    #[error("organization still has dependent workspaces or templates")]
    OrganizationInUse,
    #[error("database contains unknown role {0}")]
    UnknownRole(String),
    #[error("database contains unknown workspace state {0}")]
    UnknownWorkspaceState(String),
    #[error("database contains unknown workspace access mode {0}")]
    UnknownAccessMode(String),
    #[error("workspace was not found")]
    WorkspaceNotFound,
    #[error("workspace HTTP port must be allowed and between 1 and 65535")]
    InvalidPortMappingPort,
    #[error("workspace port mapping display name is invalid")]
    InvalidPortMappingDisplayName,
    #[error("workspace port mapping was not found")]
    PortMappingNotFound,
    #[error("workspace name and image must not be empty")]
    InvalidWorkspace,
    #[error("workspace injection references are invalid or duplicated")]
    InvalidWorkspaceInjectionRefs,
    #[error("workspace image is not enabled by the image allowlist")]
    ImageNotAllowed,
    #[error("workspace template was not found or is disabled")]
    TemplateNotFound,
    #[error("workspace template is invalid")]
    InvalidTemplate,
    #[error("plugin configuration is invalid")]
    InvalidPluginConfiguration,
    #[error("plugin configuration version changed")]
    PluginConfigurationVersionConflict,
    #[error("plugin install inspection was not found")]
    PluginInspectionNotFound,
    #[error("plugin install inspection expired")]
    PluginInspectionExpired,
    #[error("plugin package digest does not match")]
    PluginDigestMismatch,
    #[error("plugin package version changed")]
    PluginPackageVersionConflict,
    #[error("plugin capability was not declared or approved")]
    PluginCapabilityNotApproved,
    #[error("plugin package was not found")]
    PluginPackageNotFound,
    #[error("plugin UI session is invalid or expired")]
    PluginUiSessionInvalid,
    #[error("plugin storage capacity was reached")]
    PluginCapacityExceeded,
    #[error("too many active plugin inspections")]
    TooManyPluginInspections,
    #[error("workspace template must be disabled before deletion")]
    TemplateMustBeDisabled,
    #[error("workspace template is referenced by one or more workspaces")]
    TemplateInUse,
    #[error("cluster-access templates require a system administrator")]
    PrivilegedTemplateForbidden,
    #[error(transparent)]
    Quota(#[from] crate::quota::QuotaError),
    #[error(transparent)]
    Transition(#[from] crate::workspaces::TransitionError),
    #[error("idempotency scope and key must be non-empty and at most 255 bytes")]
    InvalidIdempotencyKey,
    #[error("idempotency reservation is no longer owned by this request")]
    IdempotencyReservationLost,
    #[error(transparent)]
    Crypto(#[from] crate::crypto::CryptoError),
    #[error("database contains invalid encrypted injection data")]
    InvalidEncryptedInjection,
    #[error("database contains unknown injection scope {0}")]
    UnknownInjectionScope(String),
    #[error("database contains unknown injection kind {0}")]
    UnknownInjectionKind(String),
    #[error("only organization-scoped injections may be locked")]
    InvalidInjectionLock,
    #[error("event is invalid")]
    InvalidEvent,
    #[error("web shell ticket lifetime must be between 1 and 300 seconds")]
    InvalidTicketTtl,
    #[error("secure random number generation failed")]
    RandomSource,
    #[error("webhook URL, event prefix, or signing secret is invalid")]
    InvalidWebhook,
    #[error("webhook subscription or event was not found")]
    WebhookNotFound,
    #[error("system clock is invalid")]
    Clock,
    #[error("workspace SSH identity is invalid")]
    InvalidSshIdentity,
    #[error("database snapshots can only be exported from SQLite mode")]
    ExportRequiresSqlite,
    #[error("database snapshots can only be imported into PostgreSQL mode")]
    ImportRequiresPostgres,
    #[error("snapshot format version {0} is not supported")]
    UnsupportedSnapshotVersion(u32),
    #[error(
        "snapshot belongs to installation {snapshot}, not configured installation {configured}"
    )]
    SnapshotInstallationMismatch {
        snapshot: String,
        configured: String,
    },
    #[error("snapshot schema version {0} is newer than this binary")]
    SnapshotSchemaTooNew(i64),
    #[error("snapshot is missing required table {0}")]
    SnapshotMissingTable(String),
    #[error("snapshot contains invalid dynamic plugin state")]
    InvalidPluginSnapshot,
    #[error("PostgreSQL import destination is not empty")]
    ImportDestinationNotEmpty,
}
