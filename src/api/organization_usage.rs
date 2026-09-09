use std::{collections::BTreeMap, sync::Arc};

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{auth::Permission, quota::Resources};

use super::{ApiError, AppState, auth::principal};

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct OrganizationUsageSummary {
    total_count: u64,
    requested: Resources,
    state_counts: BTreeMap<String, u64>,
    actual: ActualUsage,
    observed_at: Option<i64>,
    availability: UsageAvailability,
    coverage: UsageCoverage,
}

#[derive(Debug, Serialize, ToSchema)]
struct ActualUsage {
    cpu_millis: Option<u64>,
    memory_mib: Option<u64>,
    disk_bytes: Option<u64>,
}

#[derive(Debug, Serialize, ToSchema)]
struct UsageAvailability {
    cpu: Availability,
    memory: Availability,
    disk: Availability,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum Availability {
    Available,
    Unavailable,
    Unknown,
}

#[derive(Debug, Serialize, ToSchema)]
struct UsageCoverage {
    /// Both counts are scoped to the authenticated key; neither reveals hidden templates.
    total_workspaces: u64,
    eligible_workspaces: u64,
    template_label_coverage: TemplateLabelCoverage,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum TemplateLabelCoverage {
    Complete,
    Incomplete,
}

#[utoipa::path(
    get,
    path = "/api/v1/organizations/{organization_id}/usage-summary",
    params(("organization_id" = Uuid, Path)),
    responses(
        (status = 200, body = OrganizationUsageSummary),
        (status = 401, body = super::ErrorEnvelope),
        (status = 403, body = super::ErrorEnvelope)
    )
)]
pub(super) async fn get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(organization_id): Path<Uuid>,
) -> Result<Json<OrganizationUsageSummary>, ApiError> {
    let actor = principal(&state, &headers).await?;
    if !actor.allows(Permission::ReadWorkspace, organization_id) {
        return Err(ApiError::Forbidden);
    }
    let summary = state
        .database
        .workspace_usage_summary(organization_id, actor.allowed_template_ids.as_deref())
        .await?;
    let metrics = state
        .organization_metrics
        .fetch(
            state.config.prometheus_url.as_ref(),
            state.config.installation_id.as_str(),
            organization_id,
            actor.allowed_template_ids.as_deref(),
            summary.active_count,
            summary.total_count,
            &state.observability,
        )
        .await;
    let availability = |value: Option<u64>| {
        if value.is_some() {
            Availability::Available
        } else if !metrics.template_labels_complete {
            Availability::Unknown
        } else {
            Availability::Unavailable
        }
    };
    Ok(Json(OrganizationUsageSummary {
        total_count: summary.total_count,
        requested: summary.requested,
        state_counts: summary.state_counts,
        actual: ActualUsage {
            cpu_millis: metrics.cpu_millis,
            memory_mib: metrics.memory_mib,
            disk_bytes: metrics.disk_bytes,
        },
        observed_at: metrics.observed_at,
        availability: UsageAvailability {
            cpu: availability(metrics.cpu_millis),
            memory: availability(metrics.memory_mib),
            disk: availability(metrics.disk_bytes),
        },
        coverage: UsageCoverage {
            total_workspaces: summary.total_count,
            eligible_workspaces: summary.total_count,
            template_label_coverage: if metrics.template_labels_complete {
                TemplateLabelCoverage::Complete
            } else {
                TemplateLabelCoverage::Incomplete
            },
        },
    }))
}
