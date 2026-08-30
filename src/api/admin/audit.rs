use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use serde::Deserialize;
use utoipa::IntoParams;
use uuid::Uuid;

use crate::{
    auth::Permission,
    storage::{AuditFilter, AuditPage},
};

use crate::api::{ApiError, AppState, auth::principal};

#[derive(Debug, Deserialize, IntoParams)]
pub(in crate::api) struct AuditQuery {
    organization_id: Option<Uuid>,
    limit: Option<u32>,
    offset: Option<u64>,
    action: Option<String>,
    actor: Option<String>,
    workspace: Option<String>,
    q: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/v1/audit",
    params(AuditQuery),
    responses(
        (status = 200, body = AuditPage),
        (status = 400, body = crate::api::ErrorEnvelope),
        (status = 403, body = crate::api::ErrorEnvelope)
    )
)]
pub(in crate::api) async fn audit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AuditQuery>,
) -> Result<Json<AuditPage>, ApiError> {
    let actor = principal(&state, &headers).await?;
    match query.organization_id {
        Some(organization_id) if actor.allows(Permission::ManageOrganization, organization_id) => {}
        None if actor.system_admin => {}
        _ => return Err(ApiError::Forbidden),
    }
    Ok(Json(
        state
            .database
            .page_audit(AuditFilter {
                organization_id: query.organization_id,
                limit: query.limit.unwrap_or(50),
                offset: query.offset.unwrap_or(0),
                action: query.action,
                actor: query.actor,
                workspace: query.workspace,
                query: query.q,
            })
            .await?,
    ))
}
