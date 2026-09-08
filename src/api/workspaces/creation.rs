use crate::{
    auth::Permission,
    kubernetes::{OWNER_INSTALLATION_LABEL, WORKSPACE_ID_LABEL},
    plugins::{WorkspaceCreateContext, WorkspaceCreatePlan},
    storage::{AdmittedWorkspaceCreation, CreateWorkspace, WorkspaceTemplate},
    workspaces::Workspace,
};

use super::super::workspace_creation::validate_inline_injections;
use super::{ApiError, AppState, CreateWorkspaceRequest};

pub(super) async fn create_admitted_workspace(
    state: &AppState,
    actor: &crate::storage::Principal,
    request: &CreateWorkspaceRequest,
    template: &WorkspaceTemplate,
    now: i64,
) -> Result<Workspace, ApiError> {
    let command = &request.workspace;
    let mut final_template = template.template.clone();
    if let Some(resources) = command.resources {
        final_template.resources = resources;
        final_template
            .validate()
            .map_err(|_| crate::storage::StorageError::InvalidWorkspace)?;
    }
    preflight_product_namespace(state).await?;
    validate_inline_injections(
        state,
        command,
        template,
        &request.inline_workspace_injections,
    )
    .await?;
    state
        .plugins
        .admit_workspace_create(
            WorkspaceCreateContext {
                installation_id: state.config.installation_id.to_string(),
                actor_user_id: actor.user_id,
                organization_id: command.organization_id,
                owner_id: command.owner_id,
                template_id: command.template_id,
            },
            WorkspaceCreatePlan::from_template(&command.name, &final_template),
        )
        .await?;
    let inline = if request.inline_workspace_injections.is_empty() {
        None
    } else {
        Some((
            state
                .cipher
                .as_ref()
                .ok_or(ApiError::EncryptionUnavailable)?,
            request.inline_workspace_injections.as_slice(),
        ))
    };
    Ok(state
        .database
        .create_workspace_with_admitted_template(AdmittedWorkspaceCreation {
            command: command.clone(),
            inline_injections: inline,
            admitted_template_yaml: &template.yaml,
            allow_cluster_access: actor.may_manage_system(),
            actor_user_id: actor.user_id,
            now,
        })
        .await?)
}

async fn preflight_product_namespace(state: &AppState) -> Result<(), ApiError> {
    let Some(client) = state.kubernetes_client.clone() else {
        return Ok(());
    };
    let namespace = crate::workspace_runtime::WORKSPACE_NAMESPACE;
    let namespaces = kube::Api::<k8s_openapi::api::core::v1::Namespace>::all(client);
    let Some(existing) = namespaces
        .get_opt(namespace)
        .await
        .map_err(ApiError::Kubernetes)?
    else {
        return Ok(());
    };
    let compatible = existing.metadata.deletion_timestamp.is_none()
        && existing.metadata.labels.as_ref().is_some_and(|labels| {
            labels.get(OWNER_INSTALLATION_LABEL).map(String::as_str)
                == Some(state.config.installation_id.as_str())
                && !labels.contains_key(WORKSPACE_ID_LABEL)
        });
    if !compatible {
        return Err(ApiError::ProductNamespaceConflict);
    }
    Ok(())
}

pub(super) async fn authorize_creation(
    state: &AppState,
    actor: &crate::storage::Principal,
    command: &CreateWorkspace,
) -> Result<WorkspaceTemplate, ApiError> {
    if !actor.allows(Permission::CreateWorkspace, command.organization_id) {
        return Err(ApiError::Forbidden);
    }
    if !actor.may_use_template(command.template_id)
        || actor.has_template_restriction() && command.resources.is_some()
    {
        return Err(ApiError::Forbidden);
    }
    if command.owner_id != actor.user_id
        && !actor.allows(Permission::ManageMembers, command.organization_id)
    {
        return Err(ApiError::Forbidden);
    }
    let template = state
        .database
        .get_workspace_template(command.template_id)
        .await?;
    if !template.enabled
        || template
            .organization_id
            .is_some_and(|id| id != command.organization_id)
    {
        return Err(crate::storage::StorageError::TemplateNotFound.into());
    }
    if template.template.cluster_access && !actor.may_manage_system() {
        return Err(ApiError::Forbidden);
    }
    Ok(template)
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, convert::Infallible, net::SocketAddr};

    use axum::{
        body::Body,
        http::{Method, Request, Response, StatusCode},
    };
    use tower::service_fn;

    use super::*;
    use crate::{
        config::AppConfig,
        quota::Resources,
        storage::{CreateOrganization, CreateWorkspaceTemplate, Database},
        templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
        workspaces::AccessMode,
    };

    const TOKEN: &str = "namespace-preflight-admin-000000000000000000000";

    #[derive(Clone, Copy, Debug)]
    enum NamespaceFixture {
        Absent,
        Owned,
        WrongOwner,
        WorkspaceLabel,
        Terminating,
    }

    #[tokio::test]
    async fn product_namespace_preflight_is_fail_closed_before_workspace_insert() {
        for fixture in [
            NamespaceFixture::Absent,
            NamespaceFixture::Owned,
            NamespaceFixture::WrongOwner,
            NamespaceFixture::WorkspaceLabel,
            NamespaceFixture::Terminating,
        ] {
            let (state, actor, request, template, organization_id) = setup(fixture).await;
            let result = create_admitted_workspace(&state, &actor, &request, &template, 10).await;
            let allowed = matches!(fixture, NamespaceFixture::Absent | NamespaceFixture::Owned);
            if allowed {
                assert!(result.is_ok(), "fixture {fixture:?} should be accepted");
            } else {
                assert!(
                    matches!(result, Err(ApiError::ProductNamespaceConflict)),
                    "fixture {fixture:?} should fail with a namespace ownership conflict"
                );
            }
            assert_eq!(
                state
                    .database
                    .list_workspaces(organization_id)
                    .await
                    .unwrap()
                    .len(),
                usize::from(allowed),
                "fixture {fixture:?} persisted an unexpected workspace count"
            );
        }
    }

    async fn setup(
        fixture: NamespaceFixture,
    ) -> (
        AppState,
        crate::storage::Principal,
        CreateWorkspaceRequest,
        WorkspaceTemplate,
        uuid::Uuid,
    ) {
        let installation_id = "preflight-a"
            .parse::<crate::config::InstallationId>()
            .unwrap();
        let database = Database::connect("sqlite::memory:", installation_id.clone())
            .await
            .unwrap();
        database.migrate().await.unwrap();
        database
            .upsert_image_policy("registry.example/workspace:1", true, 1)
            .await
            .unwrap();
        let key_now = crate::api::idempotency::unix_timestamp().unwrap();
        let user = database
            .create_user_with_initial_key(
                "Preflight Admin",
                TOKEN,
                true,
                crate::auth::ApiKeyScope::initial_key_defaults(true),
                key_now + 86_400,
                key_now,
            )
            .await
            .unwrap();
        let organization = database
            .create_organization(
                CreateOrganization {
                    name: "Preflight Org".to_owned(),
                    owner_user_id: user.user_id,
                },
                2,
            )
            .await
            .unwrap();
        let template = database
            .create_workspace_template(
                CreateWorkspaceTemplate {
                    organization_id: Some(organization.id),
                    yaml: WorkspaceTemplateDocument::new(
                        "Preflight template",
                        WorkspaceTemplateSpec::standard(
                            "registry.example/workspace:1",
                            AccessMode::Internal,
                            Resources {
                                cpu_millis: 500,
                                memory_mib: 512,
                                gpu_count: 0,
                                disk_gib: 5,
                            },
                        ),
                    )
                    .to_yaml()
                    .unwrap(),
                },
                false,
                3,
            )
            .await
            .unwrap();
        let actor = database.authenticate(TOKEN).await.unwrap().unwrap();
        let request = CreateWorkspaceRequest {
            workspace: CreateWorkspace {
                organization_id: organization.id,
                owner_id: user.user_id,
                name: format!("preflight-{fixture:?}"),
                template_id: template.id,
                resources: None,
                organization_injection_refs: None,
                user_injection_refs: None,
            },
            inline_workspace_injections: Vec::new(),
        };
        let mut state = AppState::new(
            AppConfig {
                installation_id,
                listen_address: SocketAddr::from(([127, 0, 0, 1], 0)),
                database_url: "sqlite::memory:".to_owned(),
                replica_count: 1,
                instance_id: "test".to_owned(),
                ssh_public_host: None,
                internal_ssh_host: None,
                web_shell_public_origin: None,
                port_mapping_public_domain: None,
                prometheus_url: None,
                plugin_dir: None,
            },
            database,
        );
        state.set_kubernetes_client(fake_kubernetes_client(fixture));
        (state, actor, request, template, organization.id)
    }

    fn fake_kubernetes_client(fixture: NamespaceFixture) -> kube::Client {
        let service = service_fn(move |request: Request<kube::client::Body>| async move {
            assert_eq!(request.method(), Method::GET);
            assert_eq!(
                request.uri().path(),
                "/api/v1/namespaces/memeloop-workspace-control"
            );
            Ok::<_, Infallible>(namespace_response(fixture))
        });
        kube::Client::new(service, "default")
    }

    fn namespace_response(fixture: NamespaceFixture) -> Response<Body> {
        if matches!(fixture, NamespaceFixture::Absent) {
            return json_response(
                StatusCode::NOT_FOUND,
                serde_json::json!({
                    "apiVersion": "v1",
                    "kind": "Status",
                    "status": "Failure",
                    "reason": "NotFound",
                    "message": "not found",
                    "code": 404,
                }),
            );
        }
        let owner = if matches!(fixture, NamespaceFixture::WrongOwner) {
            "other"
        } else {
            "preflight-a"
        };
        let mut labels = BTreeMap::from([(OWNER_INSTALLATION_LABEL.to_owned(), owner.to_owned())]);
        if matches!(fixture, NamespaceFixture::WorkspaceLabel) {
            labels.insert(WORKSPACE_ID_LABEL.to_owned(), uuid::Uuid::nil().to_string());
        }
        let deletion_timestamp =
            matches!(fixture, NamespaceFixture::Terminating).then_some("2026-09-06T00:00:00Z");
        json_response(
            StatusCode::OK,
            serde_json::json!({
                "apiVersion": "v1",
                "kind": "Namespace",
                "metadata": {
                    "name": "memeloop-workspace-control",
                    "labels": labels,
                    "deletionTimestamp": deletion_timestamp,
                },
            }),
        )
    }

    fn json_response(status: StatusCode, value: serde_json::Value) -> Response<Body> {
        Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .body(Body::from(value.to_string()))
            .unwrap()
    }
}
