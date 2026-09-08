use std::collections::BTreeMap;

use serde_json::{Value, json};
use uuid::Uuid;

use super::super::super::{
    OWNER_INSTALLATION_LABEL, ResourceBuilder, TtydMtlsConfig, WORKSPACE_ID_LABEL,
    client::ReconcileError,
};
use crate::{
    quota::Resources,
    templates::WorkspaceTemplateSpec,
    workspace_runtime::{WORKSPACE_NAMESPACE, WorkspaceRuntimeIdentity},
    workspaces::{AccessMode, Workspace, WorkspaceState},
};

pub(super) const INSTALLATION_ID: &str = "public-a";
pub(super) const HIGRESS_NAMESPACE: &str = "higress-system";
pub(super) const CLIENT_SECRET_NAME: &str = "higress-client";
pub(super) const SERVER_SECRET_NAME: &str = "workspace-server";
pub(super) const WEB_SHELL_DOMAIN: &str = "shell.example.test";
pub(super) const FILTER_UID: &str = "filter-uid-1";

#[derive(Clone, Copy)]
pub(super) enum SecretFailure {
    MissingClient,
    EmptyClient,
    MissingCompanion,
    EmptyCompanion,
}

#[derive(Clone, Copy)]
pub(super) enum ReconcileErrorKind {
    MissingClient,
    InvalidClient,
    MissingCompanion,
    InvalidCompanion,
}

pub(super) fn assert_reconcile_error(
    result: Result<(), ReconcileError>,
    expected: ReconcileErrorKind,
) {
    match (result, expected) {
        (Err(ReconcileError::MissingTtydMtlsSecret), ReconcileErrorKind::MissingClient)
        | (Err(ReconcileError::InvalidTtydMtlsClientSecret), ReconcileErrorKind::InvalidClient)
        | (Err(ReconcileError::MissingTtydMtlsCompanion), ReconcileErrorKind::MissingCompanion)
        | (Err(ReconcileError::InvalidTtydMtlsCompanion), ReconcileErrorKind::InvalidCompanion) => {
        }
        (result, _) => panic!("unexpected reconcile result: {result:?}"),
    }
}

pub(super) fn secret_case(
    builder: &ResourceBuilder,
    failure: SecretFailure,
) -> (Option<Value>, Option<Value>) {
    let client = match failure {
        SecretFailure::MissingClient => None,
        SecretFailure::EmptyClient => Some(secret(
            HIGRESS_NAMESPACE,
            CLIENT_SECRET_NAME,
            json!({"tls.crt": "", "tls.key": ""}),
        )),
        SecretFailure::MissingCompanion | SecretFailure::EmptyCompanion => {
            Some(valid_client_secret(builder))
        }
    };
    let companion = match failure {
        SecretFailure::MissingClient | SecretFailure::MissingCompanion => None,
        SecretFailure::EmptyClient => Some(valid_companion_secret(builder)),
        SecretFailure::EmptyCompanion => Some(secret(
            HIGRESS_NAMESPACE,
            &format!("{CLIENT_SECRET_NAME}-cacert"),
            json!({"cacert": ""}),
        )),
    };
    (client, companion)
}

pub(super) fn valid_client_secret(builder: &ResourceBuilder) -> Value {
    let mtls = builder.ttyd_mtls.as_ref().unwrap();
    secret(
        HIGRESS_NAMESPACE,
        &mtls.higress_client_secret_name,
        json!({"tls.crt": "Y2VydA==", "tls.key": "a2V5"}),
    )
}

pub(super) fn valid_companion_secret(builder: &ResourceBuilder) -> Value {
    let mtls = builder.ttyd_mtls.as_ref().unwrap();
    secret(
        HIGRESS_NAMESPACE,
        &format!("{}-cacert", mtls.higress_client_secret_name),
        json!({"cacert": "Y2E="}),
    )
}

fn secret(namespace: &str, name: &str, data: Value) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": {"name": name, "namespace": namespace},
        "data": data,
    })
}

pub(super) fn workspace() -> Workspace {
    let id = Uuid::parse_str("018f0000-0000-7000-8000-000000000001").unwrap();
    Workspace {
        id,
        short_id: "8000000000000001".to_owned(),
        organization_id: Uuid::now_v7(),
        owner_id: Uuid::now_v7(),
        name: "test".to_owned(),
        template_id: None,
        runtime: WorkspaceRuntimeIdentity,
        template: WorkspaceTemplateSpec::standard(
            "example/workspace:1",
            AccessMode::Public,
            Resources {
                cpu_millis: 1_000,
                memory_mib: 1_024,
                gpu_count: 0,
                disk_gib: 10,
            },
        ),
        state: WorkspaceState::Ready,
        generation: 1,
        created_at: 1,
        updated_at: 1,
    }
}

pub(super) fn builder(mtls_enabled: bool, web_shell_domain: Option<&str>) -> ResourceBuilder {
    ResourceBuilder {
        installation_id: INSTALLATION_ID.parse().unwrap(),
        ttyd_image: "example/ttyd:1".to_owned(),
        ttyd_mtls: mtls_enabled.then(|| {
            TtydMtlsConfig::new(
                SERVER_SECRET_NAME.to_owned(),
                HIGRESS_NAMESPACE.to_owned(),
                CLIENT_SECRET_NAME.to_owned(),
            )
            .unwrap()
        }),
        higress_namespace: HIGRESS_NAMESPACE.to_owned(),
        higress_pod_labels: BTreeMap::from([("app".to_owned(), "higress-gateway".to_owned())]),
        higress_source_cidrs: Vec::new(),
        internet_egress: None,
        jump_host_namespace: "access".to_owned(),
        jump_host_pod_labels: BTreeMap::new(),
        storage_class_name: None,
        web_shell_domain: web_shell_domain.map(str::to_owned),
        port_mapping_domain: None,
        higress_gateway_name: "higress".to_owned(),
        higress_https_section_name: "https".to_owned(),
        internal_ssh_node_port_enabled: false,
    }
}

pub(super) fn owned_labels(builder: &ResourceBuilder, workspace: &Workspace) -> Value {
    labels(builder.installation_id.as_str(), workspace.id)
}

pub(super) fn foreign_labels(workspace: &Workspace) -> Value {
    labels("foreign-installation", workspace.id)
}

fn labels(installation: &str, workspace_id: Uuid) -> Value {
    let mut labels = serde_json::Map::new();
    labels.insert(OWNER_INSTALLATION_LABEL.to_owned(), json!(installation));
    labels.insert(
        WORKSPACE_ID_LABEL.to_owned(),
        json!(workspace_id.to_string()),
    );
    Value::Object(labels)
}

pub(super) fn ingress(
    builder: &ResourceBuilder,
    workspace: &Workspace,
    labels: Value,
    with_finalizer: bool,
) -> Value {
    let names = builder.runtime_names(workspace).unwrap();
    let mut metadata = json!({
        "name": names.resources.web_shell_ingress,
        "namespace": WORKSPACE_NAMESPACE,
        "labels": labels,
        "uid": "ingress-uid-1",
    });
    if with_finalizer {
        metadata["finalizers"] = json!(["cleanup.example.test/finalizer"]);
    }
    json!({
        "apiVersion": "networking.k8s.io/v1",
        "kind": "Ingress",
        "metadata": metadata,
        "spec": {},
    })
}

pub(super) fn envoy_filter(
    builder: &ResourceBuilder,
    workspace: &Workspace,
    labels: Value,
) -> Value {
    let names = builder.runtime_names(workspace).unwrap();
    json!({
        "apiVersion": "networking.istio.io/v1alpha3",
        "kind": "EnvoyFilter",
        "metadata": {
            "name": names.resources.web_shell_envoy_filter,
            "namespace": HIGRESS_NAMESPACE,
            "labels": labels,
            "uid": FILTER_UID,
        },
        "spec": {},
    })
}

pub(super) fn client_secret_path(builder: &ResourceBuilder) -> String {
    let mtls = builder.ttyd_mtls.as_ref().unwrap();
    format!(
        "/api/v1/namespaces/{}/secrets/{}",
        mtls.higress_client_secret_namespace, mtls.higress_client_secret_name
    )
}

pub(super) fn companion_secret_path(builder: &ResourceBuilder) -> String {
    let mtls = builder.ttyd_mtls.as_ref().unwrap();
    format!(
        "/api/v1/namespaces/{}/secrets/{}-cacert",
        mtls.higress_client_secret_namespace, mtls.higress_client_secret_name
    )
}

pub(super) fn filter_path(builder: &ResourceBuilder, name: &str) -> String {
    format!(
        "/apis/networking.istio.io/v1alpha3/namespaces/{}/envoyfilters/{name}",
        builder.higress_namespace
    )
}

pub(super) fn ingress_path(name: &str) -> String {
    format!("/apis/networking.k8s.io/v1/namespaces/{WORKSPACE_NAMESPACE}/ingresses/{name}")
}
