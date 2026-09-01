//! API and Higress external-auth endpoint for workspace HTTP mappings.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    auth::Permission,
    storage::{IdempotencyDecision, NewJob, PortMapping, StorageError, validate_http_port},
};

use super::{
    ApiError, AppState,
    auth::principal,
    idempotency::{
        IDEMPOTENCY_TTL_SECONDS, hash, idempotency_key, json_response, replay_response,
        unix_timestamp,
    },
};

mod auth;

pub(super) use auth::{authorize, bootstrap};

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub(super) struct CreatePortMappingRequest {
    pub internal_port: u16,
    pub display_name: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct PortMappingResponse {
    pub id: Uuid,
    pub internal_port: u16,
    pub display_name: Option<String>,
    pub status: &'static str,
    /// Stable HTTPS address, never an authorization credential.
    pub https_url: String,
    pub created_at: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct OpenPortMappingResponse {
    /// Short-lived one-use URL intended only for immediate browser navigation.
    pub bootstrap_url: String,
    pub expires_at: i64,
}

#[utoipa::path(
    get,
    path = "/api/v1/workspaces/{workspace_id}/port-mappings",
    params(("workspace_id" = Uuid, Path)),
    responses((status = 200, body = [PortMappingResponse]), (status = 403, body = super::ErrorEnvelope))
)]
pub(super) async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<Vec<PortMappingResponse>>, ApiError> {
    let actor = principal(&state, &headers).await?;
    let workspace = state.database.get_workspace(workspace_id).await?;
    if !actor.allows(Permission::ConnectWorkspace, workspace.organization_id) {
        return Err(ApiError::Forbidden);
    }
    let mappings = state.database.list_port_mappings(workspace_id).await?;
    let mut responses = Vec::with_capacity(mappings.len());
    for mapping in &mappings {
        responses.push(response(
            &state,
            mapping,
            mapping_status(&state, &workspace, mapping).await,
        )?);
    }
    Ok(Json(responses))
}

#[utoipa::path(
    post,
    path = "/api/v1/workspaces/{workspace_id}/port-mappings",
    params(("workspace_id" = Uuid, Path), ("Idempotency-Key" = String, Header)),
    request_body = CreatePortMappingRequest,
    responses((status = 201, body = PortMappingResponse), (status = 400, body = super::ErrorEnvelope), (status = 403, body = super::ErrorEnvelope))
)]
pub(super) async fn create(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(workspace_id): Path<Uuid>,
    Json(request): Json<CreatePortMappingRequest>,
) -> Result<Response, ApiError> {
    validate_http_port(request.internal_port)
        .map_err(|_| ApiError::BadRequest("port is not an allowed workspace HTTP port"))?;
    let actor = principal(&state, &headers).await?;
    let workspace = state.database.get_workspace(workspace_id).await?;
    if !actor.allows(Permission::ConnectWorkspace, workspace.organization_id) {
        return Err(ApiError::Forbidden);
    }
    if workspace.state != crate::workspaces::WorkspaceState::Ready {
        return Err(ApiError::WorkspaceNotConnectable);
    }
    mapping_domain(&state)?;
    let now = unix_timestamp()?;
    let key = idempotency_key(&headers)?;
    let request_hash = hash(&request)?;
    let scope = format!("{}:{workspace_id}:create-port-mapping", actor.user_id);
    match state
        .database
        .begin_idempotency(
            &scope,
            key,
            &request_hash,
            now,
            now + IDEMPOTENCY_TTL_SECONDS,
        )
        .await?
    {
        IdempotencyDecision::Replay(replay) => return replay_response(replay),
        IdempotencyDecision::Conflict => return Err(ApiError::IdempotencyConflict),
        IdempotencyDecision::InProgress => return Err(ApiError::IdempotencyInProgress),
        IdempotencyDecision::Reserved => {}
    }
    let result = async {
        let mapping = state
            .database
            .create_port_mapping(
                workspace.organization_id,
                workspace.id,
                request.internal_port,
                request.display_name.as_deref(),
                actor.user_id,
                now,
            )
            .await?;
        enqueue_reconcile(&state, workspace.id, workspace.generation, now).await?;
        state
            .database
            .record_audit(
                Some(actor.user_id),
                Some(workspace.organization_id),
                Some(workspace.id),
                "workspace.port_mapping.create",
                serde_json::json!({"mapping_id": mapping.id, "internal_port": mapping.internal_port}),
                now,
            )
            .await?;
        response(&state, &mapping, "provisioning")
    }
    .await;
    let response = match result {
        Ok(response) => response,
        Err(error) => {
            state
                .database
                .abandon_idempotency(&scope, key, &request_hash)
                .await?;
            return Err(error);
        }
    };
    let response_json = serde_json::to_string(&response)
        .map_err(|_| ApiError::BadRequest("response serialization failed"))?;
    state
        .database
        .finish_idempotency(
            &scope,
            key,
            &request_hash,
            StatusCode::CREATED.as_u16(),
            &response_json,
        )
        .await?;
    json_response(StatusCode::CREATED, response_json)
}

#[utoipa::path(
    post,
    path = "/api/v1/workspaces/{workspace_id}/port-mappings/{mapping_id}/open",
    params(("workspace_id" = Uuid, Path), ("mapping_id" = Uuid, Path)),
    responses((status = 200, body = OpenPortMappingResponse), (status = 403, body = super::ErrorEnvelope), (status = 404, body = super::ErrorEnvelope))
)]
pub(super) async fn open(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((workspace_id, mapping_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<OpenPortMappingResponse>, ApiError> {
    let actor = principal(&state, &headers).await?;
    let workspace = state.database.get_workspace(workspace_id).await?;
    if !actor.allows(Permission::ConnectWorkspace, workspace.organization_id) {
        return Err(ApiError::Forbidden);
    }
    if workspace.state != crate::workspaces::WorkspaceState::Ready {
        return Err(ApiError::WorkspaceNotConnectable);
    }
    let mapping = state.database.get_port_mapping(mapping_id).await?;
    if mapping.workspace_id != workspace.id {
        return Err(ApiError::Forbidden);
    }
    let now = unix_timestamp()?;
    let issued = state
        .database
        .issue_port_mapping_ticket(&mapping, actor.user_id, now)
        .await?;
    Ok(Json(OpenPortMappingResponse {
        bootstrap_url: format!(
            "{}/_mwc/bootstrap?ticket={}",
            mapping_origin(&state, &mapping)?,
            issued.ticket
        ),
        expires_at: issued.expires_at,
    }))
}

#[utoipa::path(
    delete,
    path = "/api/v1/workspaces/{workspace_id}/port-mappings/{mapping_id}",
    params(("workspace_id" = Uuid, Path), ("mapping_id" = Uuid, Path)),
    responses((status = 204), (status = 403, body = super::ErrorEnvelope), (status = 404, body = super::ErrorEnvelope))
)]
pub(super) async fn delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((workspace_id, mapping_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let actor = principal(&state, &headers).await?;
    let workspace = state.database.get_workspace(workspace_id).await?;
    if !actor.allows(Permission::ConnectWorkspace, workspace.organization_id) {
        return Err(ApiError::Forbidden);
    }
    // FK cascade revokes all tickets and sessions immediately. The reconciler
    // deletes owned Kubernetes objects; an in-flight cookie can no longer pass
    // external-auth even before that reconciliation completes.
    let now = unix_timestamp()?;
    if !state
        .database
        .delete_port_mapping(workspace_id, mapping_id)
        .await?
    {
        return Err(StorageError::PortMappingNotFound.into());
    }
    enqueue_reconcile(&state, workspace.id, workspace.generation, now).await?;
    state
        .database
        .record_audit(
            Some(actor.user_id),
            Some(workspace.organization_id),
            Some(workspace.id),
            "workspace.port_mapping.delete",
            serde_json::json!({"mapping_id": mapping_id}),
            now,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn mapping_origin(state: &AppState, mapping: &PortMapping) -> Result<String, ApiError> {
    let domain = mapping_domain(state)?;
    Ok(format!("https://p-{}.{}", mapping.id.simple(), domain))
}

fn mapping_domain(state: &AppState) -> Result<&str, ApiError> {
    state
        .config
        .port_mapping_public_domain
        .as_deref()
        .ok_or(ApiError::KubernetesUnavailable)
}

async fn enqueue_reconcile(
    state: &AppState,
    workspace_id: Uuid,
    generation: u64,
    now: i64,
) -> Result<(), ApiError> {
    state
        .database
        .enqueue_job(
            NewJob {
                kind: "reconcile_workspace".to_owned(),
                workspace_id: Some(workspace_id),
                payload: serde_json::json!({"generation": generation}),
                available_at: now,
            },
            now,
        )
        .await?;
    Ok(())
}

fn response(
    state: &AppState,
    mapping: &PortMapping,
    status: &'static str,
) -> Result<PortMappingResponse, ApiError> {
    Ok(PortMappingResponse {
        id: mapping.id,
        internal_port: mapping.internal_port,
        display_name: mapping.display_name.clone(),
        status,
        https_url: mapping_origin(state, mapping)?,
        created_at: mapping.created_at,
    })
}

async fn mapping_status(
    state: &AppState,
    workspace: &crate::workspaces::Workspace,
    mapping: &PortMapping,
) -> &'static str {
    let Some(client) = state.kubernetes_client.clone() else {
        return "provisioning";
    };
    let Ok(namespace) = state
        .config
        .installation_id
        .workspace_namespace(&workspace.short_id)
    else {
        return "failed";
    };
    let ingresses =
        kube::Api::<k8s_openapi::api::networking::v1::Ingress>::namespaced(client, &namespace);
    match ingresses
        .get_opt(&format!("port-{}", mapping.id.simple()))
        .await
    {
        Ok(Some(ingress))
            if ingress.metadata.labels.as_ref().is_some_and(|labels| {
                labels.get("workspace.memeloop.dev/port-mapping-id")
                    == Some(&mapping.id.to_string())
                    && labels.get(crate::kubernetes::OWNER_INSTALLATION_LABEL)
                        == Some(&state.config.installation_id.to_string())
            }) =>
        {
            "ready"
        }
        Ok(_) => "provisioning",
        Err(_) => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_contract_uses_internal_port_and_optional_display_name() {
        let request: CreatePortMappingRequest =
            serde_json::from_str(r#"{"internal_port":3000,"display_name":"frontend"}"#).unwrap();
        assert_eq!(request.internal_port, 3000);
        assert_eq!(request.display_name.as_deref(), Some("frontend"));
        assert!(serde_json::from_str::<CreatePortMappingRequest>(r#"{"port":3000}"#).is_err());
    }
}
