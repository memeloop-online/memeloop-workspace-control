use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use utoipa::ToSchema;

use crate::storage::StorageError;

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
    KubernetesUnavailable,
    Kubernetes(kube::Error),
    Injection(crate::injections::InjectionError),
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

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorEnvelope {
    pub error: ErrorBody,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    pub code: &'static str,
    pub message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "a valid Bearer token is required".to_owned(),
            ),
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                "forbidden",
                "the authenticated user is not allowed to perform this action".to_owned(),
            ),
            Self::BadRequest(message) => {
                (StatusCode::BAD_REQUEST, "bad_request", message.to_owned())
            }
            Self::MissingIdempotencyKey => (
                StatusCode::BAD_REQUEST,
                "missing_idempotency_key",
                "Idempotency-Key header is required".to_owned(),
            ),
            Self::IdempotencyConflict => (
                StatusCode::CONFLICT,
                "idempotency_conflict",
                "this idempotency key was already used with a different request".to_owned(),
            ),
            Self::IdempotencyInProgress => (
                StatusCode::CONFLICT,
                "idempotency_in_progress",
                "an equivalent request is still in progress".to_owned(),
            ),
            Self::EncryptionUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "encryption_unavailable",
                "injection APIs require MWC_ENCRYPTION_KEY".to_owned(),
            ),
            Self::WorkspaceNotConnectable => (
                StatusCode::CONFLICT,
                "workspace_not_connectable",
                "new Web Shell and SSH authorization requires a ready workspace".to_owned(),
            ),
            Self::KubernetesUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "kubernetes_unavailable",
                "runtime observations require Kubernetes coordination".to_owned(),
            ),
            Self::Kubernetes(error) => {
                tracing::error!(error = %error, "Kubernetes runtime observation failed");
                (StatusCode::BAD_GATEWAY, "kubernetes_error", "Kubernetes runtime observations could not be read".to_owned())
            }
            Self::Injection(error @ crate::injections::InjectionError::LockedOverride { .. }) => (
                StatusCode::CONFLICT,
                "locked_injection_conflict",
                error.to_string(),
            ),
            Self::Injection(error) => (
                StatusCode::BAD_REQUEST,
                "invalid_injection",
                error.to_string(),
            ),
            Self::Storage(StorageError::WorkspaceNotFound) => (
                StatusCode::NOT_FOUND,
                "workspace_not_found",
                "workspace was not found".to_owned(),
            ),
            Self::Storage(StorageError::OrganizationNotFound) => (
                StatusCode::NOT_FOUND,
                "organization_not_found",
                "organization was not found".to_owned(),
            ),
            Self::Storage(StorageError::Quota(error)) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "quota_exceeded",
                error.to_string(),
            ),
            Self::Storage(StorageError::Transition(error)) => (
                StatusCode::CONFLICT,
                "invalid_workspace_transition",
                error.to_string(),
            ),
            Self::Storage(StorageError::InvalidWorkspace) => (
                StatusCode::BAD_REQUEST,
                "invalid_workspace",
                "workspace name, image, or resources are invalid".to_owned(),
            ),
            Self::Storage(StorageError::InvalidWorkspaceInjectionRefs) => (
                StatusCode::BAD_REQUEST,
                "invalid_workspace_injection_refs",
                "organization and user injection references must be unique valid keys".to_owned(),
            ),
            Self::Storage(StorageError::ImageNotAllowed) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "image_not_allowed",
                "workspace image is disabled or does not satisfy Image Contract v1".to_owned(),
            ),
            Self::Storage(StorageError::TemplateNotFound) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "template_not_found",
                "workspace template is missing, disabled, outside the organization, or its values were changed".to_owned(),
            ),
            Self::Storage(StorageError::InvalidTemplate) => (
                StatusCode::BAD_REQUEST,
                "invalid_template",
                "template or image policy is invalid".to_owned(),
            ),
            Self::Storage(StorageError::InvalidWebhook) => (
                StatusCode::BAD_REQUEST,
                "invalid_webhook",
                "webhook requires a public HTTPS URL, event prefix, and a signing secret of at least 32 bytes".to_owned(),
            ),
            Self::Storage(StorageError::WebhookNotFound) => (
                StatusCode::NOT_FOUND,
                "webhook_not_found",
                "webhook subscription or event was not found".to_owned(),
            ),
            Self::Storage(StorageError::InvalidInjectionLock) => (
                StatusCode::BAD_REQUEST,
                "invalid_injection_lock",
                "only organization injections may be locked".to_owned(),
            ),
            Self::Storage(error) => {
                tracing::error!(error = %error, "API storage operation failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "the request could not be completed".to_owned(),
                )
            }
        };
        (
            status,
            Json(ErrorEnvelope {
                error: ErrorBody { code, message },
            }),
        )
            .into_response()
    }
}
