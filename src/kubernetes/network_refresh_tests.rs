use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
};

use axum::{
    body::{Body, to_bytes},
    http::{Request, Response},
};
use serde_json::{Value, json};
use tower::service_fn;

use super::*;
use crate::{
    kubernetes::{InternetEgressConfig, ResourceBuilder},
    quota::Resources,
    templates::WorkspaceTemplateSpec,
    workspace_runtime::WorkspaceRuntimeIdentity,
    workspaces::AccessMode,
};

type Requests = Arc<Mutex<Vec<(String, String, Value)>>>;

fn fixture() -> (ResourceBuilder, Workspace) {
    let builder = ResourceBuilder {
        installation_id: "refresh-test".parse().unwrap(),
        ttyd_image: "example/ttyd:1".to_owned(),
        ttyd_mtls: None,
        higress_namespace: "higress-system".to_owned(),
        higress_pod_labels: BTreeMap::new(),
        higress_source_cidrs: vec!["100.64.0.0/10".to_owned()],
        internet_egress: Some(
            InternetEgressConfig::new(
                "kube-system".to_owned(),
                BTreeMap::from([("k8s-app".to_owned(), "kube-dns".to_owned())]),
                vec![],
            )
            .unwrap(),
        ),
        jump_host_namespace: "access".to_owned(),
        jump_host_pod_labels: BTreeMap::new(),
        storage_class_name: None,
        web_shell_domain: None,
        port_mapping_domain: None,
        higress_gateway_name: "higress".to_owned(),
        higress_https_section_name: "https".to_owned(),
        internal_ssh_node_port_enabled: false,
    };
    let workspace = Workspace {
        id: Uuid::parse_str("018f0000-0000-7000-8000-000000000001").unwrap(),
        short_id: "8000000000000001".to_owned(),
        organization_id: Uuid::now_v7(),
        owner_id: Uuid::now_v7(),
        name: "refresh".to_owned(),
        template_id: None,
        runtime: WorkspaceRuntimeIdentity,
        template: WorkspaceTemplateSpec::standard(
            "example/workspace:1",
            AccessMode::Public,
            Resources {
                cpu_millis: 500,
                memory_mib: 512,
                gpu_count: 0,
                disk_gib: 1,
            },
        ),
        state: WorkspaceState::Ready,
        generation: 1,
        created_at: 1,
        updated_at: 1,
    };
    (builder, workspace)
}

fn mock(
    builder: ResourceBuilder,
    replies: Vec<(u16, Value)>,
) -> (KubernetesCoordinator, Api<NetworkPolicy>, Requests) {
    let requests = Requests::default();
    let captured = requests.clone();
    let replies = Arc::new(Mutex::new(std::collections::VecDeque::from(replies)));
    let service = service_fn(move |request: Request<kube::client::Body>| {
        let captured = captured.clone();
        let replies = replies.clone();
        async move {
            let (parts, body) = request.into_parts();
            let body = to_bytes(Body::new(body), 65536).await.unwrap();
            let payload = if body.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&body).unwrap()
            };
            captured.lock().unwrap().push((
                parts.method.to_string(),
                parts.uri.to_string(),
                payload,
            ));
            let (status, value) = replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected Kubernetes request");
            Ok::<_, Infallible>(
                Response::builder()
                    .status(status)
                    .header("content-type", "application/json")
                    .body(Body::from(value.to_string()))
                    .unwrap(),
            )
        }
    });
    let client = kube::Client::new(service, WORKSPACE_NAMESPACE);
    let api = Api::namespaced(client.clone(), WORKSPACE_NAMESPACE);
    (KubernetesCoordinator::new(client, builder), api, requests)
}

fn stale_policy(builder: &ResourceBuilder, workspace: &Workspace) -> NetworkPolicy {
    let mut policy = builder.build(workspace).unwrap().network_policy;
    policy.spec.as_mut().unwrap().ingress = Some(vec![]);
    policy.metadata.resource_version = Some("7".to_owned());
    policy
}

fn status(code: u16) -> Value {
    json!({"apiVersion":"v1","kind":"Status","status":"Failure",
        "message":"test response","reason":if code == 404 {"NotFound"} else {"Conflict"},"code":code})
}

#[tokio::test]
async fn ready_policy_refresh_only_patches_spec_with_resource_version() {
    let (builder, workspace) = fixture();
    let stale = stale_policy(&builder, &workspace);
    let desired = builder.build(&workspace).unwrap().network_policy;
    let (coordinator, api, requests) = mock(builder, vec![(200, json!(desired))]);
    assert!(
        coordinator
            .refresh_policy(&api, &workspace, &stale)
            .await
            .unwrap()
    );
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, "PATCH");
    assert!(
        requests[0]
            .1
            .contains("/networkpolicies/w-8000000000000001-ingress?")
    );
    assert_eq!(
        requests[0].2,
        json!({"metadata":{"resourceVersion":"7"},"spec":desired.spec})
    );
}

#[tokio::test]
async fn unchanged_or_foreign_policies_never_write() {
    let (builder, workspace) = fixture();
    let mut policy = builder.build(&workspace).unwrap().network_policy;
    let (coordinator, api, requests) = mock(builder, vec![]);
    assert!(
        !coordinator
            .refresh_policy(&api, &workspace, &policy)
            .await
            .unwrap()
    );
    policy
        .metadata
        .labels
        .as_mut()
        .unwrap()
        .insert(OWNER_INSTALLATION_LABEL.to_owned(), "other".to_owned());
    assert!(matches!(
        coordinator.refresh_policy(&api, &workspace, &policy).await,
        Err(NetworkRefreshError::Ownership(_))
    ));
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn concurrent_deletion_does_not_recreate_policy() {
    let (builder, workspace) = fixture();
    let stale = stale_policy(&builder, &workspace);
    let (coordinator, api, requests) = mock(builder, vec![(404, status(404))]);
    assert!(
        !coordinator
            .refresh_policy(&api, &workspace, &stale)
            .await
            .unwrap()
    );
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn conflict_rechecks_ownership_before_retrying() {
    let (builder, workspace) = fixture();
    let stale = stale_policy(&builder, &workspace);
    let mut replacement = stale.clone();
    replacement
        .metadata
        .labels
        .as_mut()
        .unwrap()
        .insert(OWNER_INSTALLATION_LABEL.to_owned(), "other".to_owned());
    let (coordinator, api, requests) =
        mock(builder, vec![(409, status(409)), (200, json!(replacement))]);
    assert!(matches!(
        coordinator.refresh_policy(&api, &workspace, &stale).await,
        Err(NetworkRefreshError::Ownership(_))
    ));
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].0, "GET");
}

#[tokio::test]
async fn policy_without_database_record_is_ignored() {
    let (builder, workspace) = fixture();
    let stale = stale_policy(&builder, &workspace);
    let database = Database::connect("sqlite::memory:", builder.installation_id.clone())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let (coordinator, _, requests) = mock(
        builder,
        vec![(
            200,
            json!({"apiVersion":"networking.k8s.io/v1","kind":"NetworkPolicyList","metadata":{},"items":[stale]}),
        )],
    );
    assert_eq!(
        coordinator
            .refresh_network_policies(&database)
            .await
            .unwrap(),
        0
    );
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn port_mapping_policy_uses_the_same_spec_only_refresh() {
    let (mut builder, workspace) = fixture();
    builder.port_mapping_domain = Some("example.test".to_owned());
    let mapping = crate::storage::PortMapping {
        id: Uuid::now_v7(),
        organization_id: workspace.organization_id,
        workspace_id: workspace.id,
        internal_port: 3000,
        display_name: None,
        created_by: workspace.owner_id,
        created_at: 1,
    };
    let (_, _, desired) = builder
        .port_mapping_resources(&workspace, &mapping)
        .unwrap()
        .unwrap();
    let mut stale = desired.clone();
    stale.metadata.resource_version = Some("9".to_owned());
    stale.spec.as_mut().unwrap().ingress = Some(vec![]);
    let (coordinator, api, requests) = mock(builder, vec![(200, json!(desired))]);
    let policies = coordinator
        .desired_network_policies(&workspace, std::slice::from_ref(&mapping))
        .unwrap();
    assert_eq!(policies.len(), 2);
    assert_eq!(policies[1], desired);
    assert!(
        coordinator
            .patch_network_policy(&api, workspace.id, &stale, &desired)
            .await
            .unwrap()
    );
    let recorded = requests.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].0, "PATCH");
    assert!(
        recorded[0]
            .1
            .contains(desired.metadata.name.as_deref().unwrap())
    );
    assert_eq!(
        recorded[0].2,
        json!({"metadata":{"resourceVersion":"9"},"spec":desired.spec})
    );
    drop(recorded);

    let mut foreign = mapping;
    foreign.organization_id = Uuid::now_v7();
    assert_eq!(
        coordinator
            .desired_network_policies(&workspace, &[foreign])
            .unwrap()
            .len(),
        1
    );
}
