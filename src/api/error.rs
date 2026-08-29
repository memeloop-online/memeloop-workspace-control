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
    Plugin(crate::plugins::PluginError),
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

impl ApiError {
    fn response_parts(self) -> ErrorResponse {
        match self {
            Self::Unauthorized => response(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "a valid Bearer token is required",
            ),
            Self::Forbidden => response(
                StatusCode::FORBIDDEN,
                "forbidden",
                "the authenticated user is not allowed to perform this action",
            ),
            Self::BadRequest(message) => response(StatusCode::BAD_REQUEST, "bad_request", message),
            Self::MissingIdempotencyKey => response(
                StatusCode::BAD_REQUEST,
                "missing_idempotency_key",
                "Idempotency-Key header is required",
            ),
            Self::IdempotencyConflict => response(
                StatusCode::CONFLICT,
                "idempotency_conflict",
                "this idempotency key was already used with a different request",
            ),
            Self::IdempotencyInProgress => response(
                StatusCode::CONFLICT,
                "idempotency_in_progress",
                "an equivalent request is still in progress",
            ),
            Self::EncryptionUnavailable => response(
                StatusCode::SERVICE_UNAVAILABLE,
                "encryption_unavailable",
                "injection APIs require MWC_ENCRYPTION_KEY",
            ),
            Self::WorkspaceNotConnectable => response(
                StatusCode::CONFLICT,
                "workspace_not_connectable",
                "new Web Shell and SSH authorization requires a ready workspace",
            ),
            Self::KubernetesUnavailable => response(
                StatusCode::SERVICE_UNAVAILABLE,
                "kubernetes_unavailable",
                "runtime observations require Kubernetes coordination",
            ),
            Self::Kubernetes(error) => {
                tracing::error!(error = %error, "Kubernetes runtime observation failed");
                response(
                    StatusCode::BAD_GATEWAY,
                    "kubernetes_error",
                    "Kubernetes runtime observations could not be read",
                )
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
            Self::Plugin(crate::plugins::PluginError::NotFound) => response(
                StatusCode::NOT_FOUND,
                "plugin_not_found",
                "the plugin is not loaded",
            ),
            Self::Plugin(crate::plugins::PluginError::InvalidConfiguration) => response(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_plugin_configuration",
                "the plugin configuration does not satisfy its current schema",
            ),
            Self::Plugin(crate::plugins::PluginError::ConfigurationVersionConflict) => response(
                StatusCode::CONFLICT,
                "plugin_configuration_version_conflict",
                "the plugin configuration version changed",
            ),
            Self::Plugin(crate::plugins::PluginError::AdmissionDenied {
                plugin_id,
                decision_code,
            }) => {
                tracing::warn!(%plugin_id, %decision_code, "workspace create policy denied request");
                response(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "plugin_admission_denied",
                    "workspace creation was rejected by an installed policy",
                )
            }
            Self::Plugin(error) => {
                tracing::error!(error = %error, "plugin execution failed closed");
                response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "plugin_execution_failed",
                    "workspace creation policy could not be evaluated",
                )
            }
            Self::Storage(error) => storage_response(error),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = self.response_parts();
        (
            status,
            Json(ErrorEnvelope {
                error: ErrorBody { code, message },
            }),
        )
            .into_response()
    }
}

fn storage_response(error: StorageError) -> ErrorResponse {
    match error {
        StorageError::WorkspaceNotFound => response(
            StatusCode::NOT_FOUND,
            "workspace_not_found",
            "workspace was not found",
        ),
        StorageError::OrganizationNotFound => response(
            StatusCode::NOT_FOUND,
            "organization_not_found",
            "organization was not found",
        ),
        StorageError::Quota(error) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "quota_exceeded",
            error.to_string(),
        ),
        StorageError::Transition(error) => (
            StatusCode::CONFLICT,
            "invalid_workspace_transition",
            error.to_string(),
        ),
        StorageError::InvalidWorkspace => response(
            StatusCode::BAD_REQUEST,
            "invalid_workspace",
            "workspace name, image, or resources are invalid",
        ),
        StorageError::InvalidWorkspaceInjectionRefs => response(
            StatusCode::BAD_REQUEST,
            "invalid_workspace_injection_refs",
            "organization and user injection references must be unique valid keys",
        ),
        StorageError::ImageNotAllowed => response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "image_not_allowed",
            "workspace image is disabled or does not satisfy Image Contract v1",
        ),
        StorageError::TemplateNotFound => response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "template_not_found",
            "workspace template is missing, disabled, outside the organization, or its values were changed",
        ),
        StorageError::InvalidTemplate => response(
            StatusCode::BAD_REQUEST,
            "invalid_template",
            "template or image policy is invalid",
        ),
        StorageError::InvalidPluginConfiguration => response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_plugin_configuration",
            "the plugin configuration is invalid",
        ),
        StorageError::PluginConfigurationVersionConflict => response(
            StatusCode::CONFLICT,
            "plugin_configuration_version_conflict",
            "the plugin configuration version changed",
        ),
        StorageError::TemplateMustBeDisabled => response(
            StatusCode::CONFLICT,
            "template_must_be_disabled",
            "disable the workspace template before deleting it",
        ),
        StorageError::TemplateInUse => response(
            StatusCode::CONFLICT,
            "template_in_use",
            "the workspace template is still referenced by a workspace",
        ),
        StorageError::PrivilegedTemplateForbidden => response(
            StatusCode::FORBIDDEN,
            "privileged_template_forbidden",
            "cluster-access templates require a system administrator",
        ),
        StorageError::InvalidWebhook => response(
            StatusCode::BAD_REQUEST,
            "invalid_webhook",
            "webhook requires a public HTTPS URL, event prefix, and a signing secret of at least 32 bytes",
        ),
        StorageError::WebhookNotFound => response(
            StatusCode::NOT_FOUND,
            "webhook_not_found",
            "webhook subscription or event was not found",
        ),
        StorageError::InvalidInjectionLock => response(
            StatusCode::BAD_REQUEST,
            "invalid_injection_lock",
            "only organization injections may be locked",
        ),
        error => {
            tracing::error!(error = %error, "API storage operation failed");
            response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "the request could not be completed",
            )
        }
    }
}

fn response(status: StatusCode, code: &'static str, message: &str) -> ErrorResponse {
    (status, code, message.to_owned())
}
