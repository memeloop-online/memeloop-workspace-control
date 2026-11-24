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
    storage::{IdempotencyDecision, PortMapping, StorageError},
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

pub(super) use auth::authorize;

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub(super) struct CreatePortMappingRequest {
    pub internal_port: u16,
    pub display_name: Option<String>,
    pub subdomain: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(super) struct PortMappingResponse {
    pub id: Uuid,
    pub internal_port: u16,
    pub display_name: Option<String>,
    pub subdomain: String,
    pub status: &'static str,
    /// Whether this mapping is owned by the workspace controller rather than
    /// the user. Controller-owned mappings cannot be created or deleted by the
    /// port-mapping API.
    pub managed: bool,
    pub purpose: PortMappingPurpose,
    /// Stable HTTPS address, never an authorization credential.
    pub https_url: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum PortMappingPurpose {
    User,
    Desktop,
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
    if !actor.allows(Permission::ConnectWorkspace, workspace.organization_id)
        || !actor.may_access_workspace_template(workspace.template_id)
    {
        return Err(ApiError::Forbidden);
    }
    let mappings = state.database.list_port_mappings(workspace_id).await?;
    let mut responses = Vec::with_capacity(mappings.len());
    for mapping in &mappings {
        responses.push(response(
            &state,
            mapping,
            mapping_status(&state, &workspace, mapping).await,
            purpose_for(&workspace, mapping),
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
    let actor = principal(&state, &headers).await?;
    let workspace = state.database.get_workspace(workspace_id).await?;
    if !actor.allows(Permission::ConnectWorkspace, workspace.organization_id)
        || !actor.may_access_workspace_template(workspace.template_id)
    {
        return Err(ApiError::Forbidden);
    }
    if workspace.state != crate::workspaces::WorkspaceState::Ready {
        return Err(ApiError::WorkspaceNotConnectable);
    }
    if workspace
        .template
        .desktop
        .as_ref()
        .is_some_and(|desktop| desktop.internal_port == request.internal_port)
    {
        return Err(ApiError::BadRequest(
            "the template-owned desktop port mapping cannot be created by users",
        ));
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
                request.subdomain.as_deref(),
                actor.user_id,
                now,
            )
            .await?;
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
        response(
            &state,
            &mapping,
            "provisioning",
            PortMappingPurpose::User,
        )
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
    put,
    path = "/api/v1/workspaces/{workspace_id}/port-mappings/{mapping_id}",
    params(("workspace_id" = Uuid, Path), ("mapping_id" = Uuid, Path)),
    request_body = CreatePortMappingRequest,
    responses((status = 200, body = PortMappingResponse), (status = 400, body = super::ErrorEnvelope), (status = 403, body = super::ErrorEnvelope), (status = 404, body = super::ErrorEnvelope), (status = 409, body = super::ErrorEnvelope))
)]
pub(super) async fn update(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((workspace_id, mapping_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<CreatePortMappingRequest>,
) -> Result<Json<PortMappingResponse>, ApiError> {
    let actor = principal(&state, &headers).await?;
    let workspace = state.database.get_workspace(workspace_id).await?;
    if !actor.allows(Permission::ConnectWorkspace, workspace.organization_id)
        || !actor.may_access_workspace_template(workspace.template_id)
    {
        return Err(ApiError::Forbidden);
    }
    let current = state.database.get_port_mapping(mapping_id).await?;
    if current.workspace_id != workspace_id {
        return Err(StorageError::PortMappingNotFound.into());
    }
    if is_desktop_mapping(&workspace, &current)
        || workspace
            .template
            .desktop
            .as_ref()
            .is_some_and(|desktop| desktop.internal_port == request.internal_port)
    {
        return Err(ApiError::BadRequest(
            "the template-owned desktop port mapping cannot be edited by users",
        ));
    }
    let now = unix_timestamp()?;
    let mapping = state
        .database
        .update_port_mapping(
            workspace_id,
            mapping_id,
            request.internal_port,
            request.display_name.as_deref(),
            request.subdomain.as_deref(),
            now,
        )
        .await?;
    state
        .database
        .record_audit(
            Some(actor.user_id),
            Some(workspace.organization_id),
            Some(workspace.id),
            "workspace.port_mapping.update",
            serde_json::json!({"mapping_id": mapping.id, "internal_port": mapping.internal_port}),
            now,
        )
        .await?;
    let status = if (current.internal_port != mapping.internal_port
        || current.subdomain != mapping.subdomain)
        && workspace.state == crate::workspaces::WorkspaceState::Ready
    {
        "provisioning"
    } else {
        mapping_status(&state, &workspace, &mapping).await
    };
    Ok(Json(response(
        &state,
        &mapping,
        status,
        PortMappingPurpose::User,
    )?))
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
    if !actor.allows(Permission::ConnectWorkspace, workspace.organization_id)
        || !actor.may_access_workspace_template(workspace.template_id)
    {
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
    if !actor.allows(Permission::ConnectWorkspace, workspace.organization_id)
        || !actor.may_access_workspace_template(workspace.template_id)
    {
        return Err(ApiError::Forbidden);
    }
    let mapping = state.database.get_port_mapping(mapping_id).await?;
    if mapping.workspace_id != workspace_id {
        return Err(StorageError::PortMappingNotFound.into());
    }
    if is_desktop_mapping(&workspace, &mapping) {
        return Err(ApiError::BadRequest(
            "the template-owned desktop port mapping cannot be deleted by users",
        ));
    }
    // FK cascade revokes all tickets and sessions immediately. The reconciler
    // deletes owned Kubernetes objects; an in-flight cookie can no longer pass
    // external-auth even before that reconciliation completes.
    let now = unix_timestamp()?;
    if !state
        .database
        .delete_port_mapping(workspace_id, mapping_id, now)
        .await?
    {
        return Err(StorageError::PortMappingNotFound.into());
    }
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
    Ok(format!("https://{}", mapping.hostname(domain)))
}

pub(super) fn mapping_https_url(
    state: &AppState,
    mapping: &PortMapping,
) -> Result<String, ApiError> {
    mapping_origin(state, mapping)
}

fn mapping_domain(state: &AppState) -> Result<&str, ApiError> {
    state
        .config
        .port_mapping_public_domain
        .as_deref()
        .ok_or(ApiError::KubernetesUnavailable)
}

pub(super) fn response(
    state: &AppState,
    mapping: &PortMapping,
    status: &'static str,
    purpose: PortMappingPurpose,
) -> Result<PortMappingResponse, ApiError> {
    Ok(PortMappingResponse {
        id: mapping.id,
        internal_port: mapping.internal_port,
        display_name: mapping.display_name.clone(),
        subdomain: mapping.subdomain.clone(),
        status,
        managed: matches!(purpose, PortMappingPurpose::Desktop),
        purpose,
        https_url: mapping_origin(state, mapping)?,
        created_at: mapping.created_at,
    })
}

pub(super) async fn mapping_status(
    state: &AppState,
    workspace: &crate::workspaces::Workspace,
    mapping: &PortMapping,
) -> &'static str {
    if workspace.state != crate::workspaces::WorkspaceState::Ready {
        return workspace.state.as_str();
    }
    let Some(client) = state.kubernetes_client.clone() else {
        return "provisioning";
    };
    let desktop_runtime = if is_desktop_mapping(workspace, mapping) {
        let Ok(runtime) = crate::workspace_runtime::WorkspaceRuntimeNames::for_workspace(
            &state.config.installation_id,
            &workspace.runtime,
            &workspace.short_id,
        ) else {
            return "failed";
        };
        Some(runtime)
    } else {
        None
    };
    let stateful_sets = kube::Api::<k8s_openapi::api::apps::v1::StatefulSet>::namespaced(
        client.clone(),
        workspace.runtime.namespace(),
    );
    let desktop_ready = async {
        match desktop_runtime {
            Some(runtime) => {
                stateful_sets
                    .get_opt(&runtime.resources.stateful_set)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|stateful_set| stateful_set.status)
                    .and_then(|status| status.ready_replicas)
                    == Some(1)
            }
            None => true,
        }
    };
    let ingresses = kube::Api::<k8s_openapi::api::networking::v1::Ingress>::namespaced(
        client,
        workspace.runtime.namespace(),
    );
    let ingress_name = format!("port-{}", mapping.id.simple());
    let (desktop_ready, ingress) = tokio::join!(desktop_ready, ingresses.get_opt(&ingress_name));
    if !desktop_ready {
        return "provisioning";
    }
    match ingress {
        Ok(Some(ingress))
            if ingress.metadata.labels.as_ref().is_some_and(|labels| {
                labels.get("workspace.memeloop.dev/port-mapping-id")
                    == Some(&mapping.id.to_string())
                    && labels.get(crate::kubernetes::OWNER_INSTALLATION_LABEL)
                        == Some(&state.config.installation_id.to_string())
                    && labels.get(crate::kubernetes::WORKSPACE_ID_LABEL)
                        == Some(&workspace.id.to_string())
            }) && ingress
                .spec
                .as_ref()
                .and_then(|spec| spec.rules.as_ref())
                .is_some_and(|rules| {
                    rules
                        .iter()
                        .filter(|rule| {
                            state
                                .config
                                .port_mapping_public_domain
                                .as_deref()
                                .is_some_and(|domain| {
                                    rule.host.as_deref() == Some(mapping.hostname(domain).as_str())
                                })
                        })
                        .filter_map(|rule| rule.http.as_ref())
                        .flat_map(|http| &http.paths)
                        .any(|path| {
                            path.backend.service.as_ref().is_some_and(|service| {
                                service.name == ingress_name
                                    && service.port.as_ref().and_then(|port| port.number)
                                        == Some(i32::from(mapping.internal_port))
                            })
                        })
                }) =>
        {
            "ready"
        }
        Ok(_) => "provisioning",
        Err(_) => "failed",
    }
}

pub(super) fn purpose_for(
    workspace: &crate::workspaces::Workspace,
    mapping: &PortMapping,
) -> PortMappingPurpose {
    if is_desktop_mapping(workspace, mapping) {
        PortMappingPurpose::Desktop
    } else {
        PortMappingPurpose::User
    }
}

fn is_desktop_mapping(workspace: &crate::workspaces::Workspace, mapping: &PortMapping) -> bool {
    mapping.workspace_id == workspace.id
        && workspace
            .template
            .desktop
            .as_ref()
            .is_some_and(|desktop| desktop.internal_port == mapping.internal_port)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desktop_workspace() -> crate::workspaces::Workspace {
        let id = Uuid::now_v7();
        let mut template = crate::templates::WorkspaceTemplateSpec::standard(
            "registry.example/dev:latest",
            crate::workspaces::AccessMode::Internal,
            crate::quota::Resources {
                cpu_millis: 1_000,
                memory_mib: 1_024,
                gpu_count: 0,
                disk_gib: 20,
            },
        );
        template.desktop = Some(crate::templates::DesktopEndpoint {
            internal_port: 6901,
            display_name: Some("Browser desktop".to_owned()),
        });
        crate::workspaces::Workspace {
            id,
            short_id: crate::workspace_runtime::workspace_short_id_for(id),
            organization_id: Uuid::now_v7(),
            owner_id: Uuid::now_v7(),
            name: "desktop".to_owned(),
            template_id: None,
            node_pool: "default".to_owned(),
            runtime: crate::workspace_runtime::WorkspaceRuntimeIdentity,
            home_volume_binding: None,
            scratch_volume_binding: None,
            template,
            state: crate::workspaces::WorkspaceState::Ready,
            generation: 1,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn create_contract_uses_internal_port_and_optional_display_name() {
        let request: CreatePortMappingRequest =
            serde_json::from_str(r#"{"internal_port":3000,"display_name":"frontend"}"#).unwrap();
        assert_eq!(request.internal_port, 3000);
        assert_eq!(request.display_name.as_deref(), Some("frontend"));
        assert!(serde_json::from_str::<CreatePortMappingRequest>(r#"{"port":3000}"#).is_err());
    }

    #[test]
    fn desktop_mappings_are_managed_from_the_workspace_snapshot() {
        let workspace = desktop_workspace();
        let desktop = PortMapping {
            id: Uuid::now_v7(),
            organization_id: workspace.organization_id,
            workspace_id: workspace.id,
            internal_port: 6901,
            display_name: Some("Browser desktop".to_owned()),
            subdomain: "desktop-test".to_owned(),
            created_by: workspace.owner_id,
            created_at: 1,
        };
        let application = PortMapping {
            internal_port: 3000,
            ..desktop.clone()
        };
        assert!(matches!(
            purpose_for(&workspace, &desktop),
            PortMappingPurpose::Desktop
        ));
        assert!(matches!(
            purpose_for(&workspace, &application),
            PortMappingPurpose::User
        ));
    }
}
