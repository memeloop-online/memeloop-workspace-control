use thiserror::Error;

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("plugin directory could not be loaded")]
    Package,
    #[error("plugin package is invalid: {0}")]
    InvalidPackage(String),
    #[error("plugin was not found")]
    NotFound,
    #[error("plugin configuration is invalid")]
    InvalidConfiguration,
    #[error("plugin API request is invalid")]
    InvalidApiRequest,
    #[error("plugin API middleware rejected the request")]
    MiddlewareDenied,
    #[error("plugin configuration version changed")]
    ConfigurationVersionConflict,
    #[error("workspace creation was rejected by an admission plugin")]
    AdmissionDenied {
        plugin_id: String,
        decision_code: String,
    },
    #[error("workspace admission plugin execution failed")]
    ExecutionFailed,
    #[error("plugin runtime is unavailable")]
    RuntimeUnavailable,
    #[error(transparent)]
    Storage(#[from] crate::storage::StorageError),
}

impl PluginError {
    pub(super) fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidPackage(message.into())
    }
}
