use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use utoipa::ToSchema;

use crate::storage::StorageError;

mod plugin;

use plugin::{plugin_distribution_response, plugin_response};

#[derive(Debug)]
pub enum ApiError {
    Unauthorized,
    Forbidden,
    BadRequest(&'static str),
    MissingIdempotencyKey,
    IdempotencyConflict,
    IdempotencyInProgress,
    EncryptionUnavailable,
    WorkspaceNotConnectable,
    SharedNamespaceConflict,
    KubernetesUnavailable,
    Kubernetes(kube::Error),
    Injection(crate::injections::InjectionError),
    Plugin(crate::plugins::PluginError),
    PluginDistribution(&'static str),
    Storage(StorageError),
}

impl From<StorageError> for ApiError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<crate::injections::InjectionError> for ApiError {
    fn from(error: crate::injections::InjectionError) -> Self {
        Self::Injection(error)
    }
}

impl From<crate::plugins::PluginError> for ApiError {
    fn from(error: crate::plugins::PluginError) -> Self {
        Self::Plugin(error)
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorEnvelope {
    pub error: ErrorBody,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    pub code: &'static str,
    pub message: String,
}

type ErrorResponse = (StatusCode, &'static str, String);

mod responses;

use responses::{injection_response, operational_response, storage_response};

impl ApiError {
    fn response_parts(self) -> ErrorResponse {
        match self {
            error @ (Self::Unauthorized
            | Self::Forbidden
            | Self::BadRequest(_)
            | Self::MissingIdempotencyKey
            | Self::IdempotencyConflict
            | Self::IdempotencyInProgress
            | Self::EncryptionUnavailable
            | Self::WorkspaceNotConnectable
            | Self::SharedNamespaceConflict
            | Self::KubernetesUnavailable
            | Self::Kubernetes(_)) => operational_response(error),
            Self::Injection(error) => injection_response(error),
            Self::Plugin(error) => plugin_response(error),
            Self::PluginDistribution(code) => plugin_distribution_response(code),
            Self::Storage(error) => storage_response(error),
        }
    }
}
