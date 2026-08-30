use axum::http::StatusCode;

use super::{ErrorResponse, response};

pub(super) fn plugin_response(error: crate::plugins::PluginError) -> ErrorResponse {
    match error {
        crate::plugins::PluginError::NotFound => response(
            StatusCode::NOT_FOUND,
            "plugin_not_found",
            "the plugin is not loaded",
        ),
        crate::plugins::PluginError::InvalidConfiguration => response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_plugin_configuration",
            "the plugin configuration does not satisfy its current schema",
        ),
        crate::plugins::PluginError::ConfigurationVersionConflict => response(
            StatusCode::CONFLICT,
            "plugin_configuration_version_conflict",
            "the plugin configuration version changed",
        ),
        crate::plugins::PluginError::AdmissionDenied {
            plugin_id,
            decision_code,
        } => {
            tracing::warn!(%plugin_id, %decision_code, "workspace create policy denied request");
            response(
                StatusCode::UNPROCESSABLE_ENTITY,
                "plugin_admission_denied",
                "workspace creation was rejected by an installed policy",
            )
        }
        error => {
            tracing::error!(error = %error, "plugin execution failed closed");
            response(
                StatusCode::SERVICE_UNAVAILABLE,
                "plugin_execution_failed",
                "workspace creation policy could not be evaluated",
            )
        }
    }
}

pub(super) fn plugin_distribution_response(code: &'static str) -> ErrorResponse {
    let status = match code {
        "plugin_inspection_not_found" => StatusCode::NOT_FOUND,
        "plugin_inspection_expired"
        | "plugin_digest_mismatch"
        | "plugin_package_version_conflict" => StatusCode::CONFLICT,
        "plugin_download_failed" | "plugin_runtime_reload_failed" => StatusCode::BAD_GATEWAY,
        "plugin_capability_not_approved" => StatusCode::UNPROCESSABLE_ENTITY,
        "plugin_middleware_denied" => StatusCode::FORBIDDEN,
        _ => StatusCode::BAD_REQUEST,
    };
    response(
        status,
        code,
        "the plugin package operation could not be completed",
    )
}
