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
            | Self::KubernetesUnavailable
            | Self::Kubernetes(_)) => operational_response(error),
            Self::Injection(error) => injection_response(error),
            Self::Plugin(error) => plugin_response(error),
            Self::PluginDistribution(code) => plugin_distribution_response(code),
            Self::Storage(error) => storage_response(error),
        }
    }
}

fn operational_response(error: ApiError) -> ErrorResponse {
    match error {
        ApiError::Unauthorized => response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "a valid Bearer token is required",
        ),
        ApiError::Forbidden => response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "the authenticated user is not allowed to perform this action",
        ),
        ApiError::BadRequest(message) => response(StatusCode::BAD_REQUEST, "bad_request", message),
        ApiError::MissingIdempotencyKey => response(
            StatusCode::BAD_REQUEST,
            "missing_idempotency_key",
            "Idempotency-Key header is required",
        ),
        ApiError::IdempotencyConflict => response(
            StatusCode::CONFLICT,
            "idempotency_conflict",
            "this idempotency key was already used with a different request",
        ),
        ApiError::IdempotencyInProgress => response(
            StatusCode::CONFLICT,
            "idempotency_in_progress",
            "an equivalent request is still in progress",
        ),
        ApiError::EncryptionUnavailable => response(
            StatusCode::SERVICE_UNAVAILABLE,
            "encryption_unavailable",
            "injection APIs require MWC_ENCRYPTION_KEY",
        ),
        ApiError::WorkspaceNotConnectable => response(
            StatusCode::CONFLICT,
            "workspace_not_connectable",
            "new Web Shell and SSH authorization requires a ready workspace",
        ),
        ApiError::KubernetesUnavailable => response(
            StatusCode::SERVICE_UNAVAILABLE,
            "kubernetes_unavailable",
            "runtime observations require Kubernetes coordination",
        ),
        ApiError::Kubernetes(error) => {
            tracing::error!(error = %error, "Kubernetes runtime observation failed");
            response(
                StatusCode::BAD_GATEWAY,
                "kubernetes_error",
                "Kubernetes runtime observations could not be read",
            )
        }
        _ => unreachable!("operational_response only accepts operational API errors"),
    }
}

fn injection_response(error: crate::injections::InjectionError) -> ErrorResponse {
    match error {
        error @ crate::injections::InjectionError::LockedOverride { .. } => (
            StatusCode::CONFLICT,
            "locked_injection_conflict",
            error.to_string(),
        ),
        error => (
            StatusCode::BAD_REQUEST,
            "invalid_injection",
            error.to_string(),
        ),
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
    if let Some(response) = identity_storage_response(&error) {
        return response;
    }
    if let Some(response) = plugin_storage_response(&error) {
        return response;
    }
    if let Some(response) = workspace_storage_response(&error) {
        return response;
    }
    match error {
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

fn workspace_storage_response(error: &StorageError) -> Option<ErrorResponse> {
    Some(match error {
        StorageError::WorkspaceNotFound => response(
            StatusCode::NOT_FOUND,
            "workspace_not_found",
            "workspace was not found",
        ),
        StorageError::PortMappingNotFound => response(
            StatusCode::NOT_FOUND,
            "port_mapping_not_found",
            "workspace port mapping was not found",
        ),
        StorageError::InvalidPortMappingPort => response(
            StatusCode::BAD_REQUEST,
            "invalid_port_mapping_port",
            "choose port 80, 443, or an application port from 1024 through 65535",
        ),
        StorageError::InvalidPortMappingDisplayName => response(
            StatusCode::BAD_REQUEST,
            "invalid_port_mapping_display_name",
            "port mapping display name must be at most 80 characters",
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
        _ => return None,
    })
}

fn identity_storage_response(error: &StorageError) -> Option<ErrorResponse> {
    Some(match error {
        StorageError::UserNotFound => response(
            StatusCode::NOT_FOUND,
            "user_not_found",
            "the user was not found",
        ),
        StorageError::InvalidUserProfile => response(
            StatusCode::BAD_REQUEST,
            "invalid_user_profile",
            "display name must be 1-80 characters and custom avatars must be uploaded PNG, JPEG, or WebP images",
        ),
        StorageError::InvalidApiKey => response(
            StatusCode::BAD_REQUEST,
            "invalid_api_key",
            "API keys require a 1-80 character name, explicit non-wildcard scopes, and an expiration within 365 days",
        ),
        StorageError::InvalidApiKeyQuery => response(
            StatusCode::BAD_REQUEST,
            "invalid_api_key_query",
            "the API key pagination cursor is invalid",
        ),
        StorageError::InvalidOrganizationMembership => response(
            StatusCode::BAD_REQUEST,
            "invalid_organization_membership",
            "organization memberships require a member or organization administrator role",
        ),
        StorageError::ApiKeyNotFound => response(
            StatusCode::NOT_FOUND,
            "api_key_not_found",
            "the API key was not found",
        ),
        StorageError::SelfApiKeyAdministration => response(
            StatusCode::BAD_REQUEST,
            "self_api_key_administration",
            "use the personal API-key endpoint when revoking your own key",
        ),
        StorageError::LastApiKey => response(
            StatusCode::CONFLICT,
            "last_api_key",
            "create another active API key with the required recovery permissions before revoking this key",
        ),
        StorageError::TooManyApiKeys => response(
            StatusCode::CONFLICT,
            "too_many_api_keys",
            "revoke an API key before creating another one",
        ),
        StorageError::LastSystemAdmin => response(
            StatusCode::CONFLICT,
            "last_system_admin",
            "create or promote another active system administrator before disabling or demoting this user",
        ),
        StorageError::LastOrganizationAdmin => response(
            StatusCode::CONFLICT,
            "last_organization_admin",
            "create or promote another organization administrator before demoting or removing this member",
        ),
        StorageError::OrganizationInUse => response(
            StatusCode::CONFLICT,
            "organization_in_use",
            "delete or move dependent workspaces and templates before deleting this organization",
        ),
        StorageError::InvalidAuditQuery => response(
            StatusCode::BAD_REQUEST,
            "invalid_audit_query",
            "audit pagination or filters are invalid",
        ),
        _ => return None,
    })
}

fn plugin_storage_response(error: &StorageError) -> Option<ErrorResponse> {
    Some(match error {
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
        StorageError::PluginInspectionNotFound => response(
            StatusCode::NOT_FOUND,
            "plugin_inspection_not_found",
            "the plugin inspection was not found",
        ),
        StorageError::PluginInspectionExpired => response(
            StatusCode::CONFLICT,
            "plugin_inspection_expired",
            "the plugin inspection expired",
        ),
        StorageError::PluginDigestMismatch => response(
            StatusCode::CONFLICT,
            "plugin_digest_mismatch",
            "the plugin package digest changed",
        ),
        StorageError::PluginPackageVersionConflict => response(
            StatusCode::CONFLICT,
            "plugin_package_version_conflict",
            "the installed plugin version changed",
        ),
        StorageError::PluginCapabilityNotApproved => response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "plugin_capability_not_approved",
            "a requested plugin capability was not declared",
        ),
        StorageError::PluginPackageNotFound => response(
            StatusCode::NOT_FOUND,
            "plugin_not_found",
            "the plugin package was not found",
        ),
        StorageError::PluginUiSessionInvalid => response(
            StatusCode::UNAUTHORIZED,
            "plugin_ui_session_invalid",
            "the plugin UI session is invalid or expired",
        ),
        StorageError::PluginCapacityExceeded => response(
            StatusCode::CONFLICT,
            "plugin_capacity_exceeded",
            "the installation plugin capacity was reached",
        ),
        StorageError::TooManyPluginInspections => response(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_plugin_inspections",
            "finish or wait for existing plugin inspections before uploading another package",
        ),
        _ => return None,
    })
}

fn response(status: StatusCode, code: &'static str, message: &str) -> ErrorResponse {
    (status, code, message.to_owned())
}
