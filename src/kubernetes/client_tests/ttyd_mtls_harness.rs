use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
};

use axum::{
    body::Body,
    http::{Method, Request, Response, StatusCode},
};
use http_body_util::BodyExt;
use kube::client::Body as KubeBody;
use serde_json::{Value, json};
use tower::service_fn;

use super::super::super::{ResourceBuilder, client::KubernetesCoordinator};
use super::support::{client_secret_path, companion_secret_path, filter_path, ingress_path};
use crate::{workspace_runtime::WORKSPACE_NAMESPACE, workspaces::Workspace};

#[derive(Default)]
pub(super) struct FakeKubeConfig {
    namespace_exists: bool,
    ingress: Option<Value>,
    filter: Option<Value>,
    client_secret: Option<Value>,
    companion_secret: Option<Value>,
    retain_ingress_on_delete: bool,
}

impl FakeKubeConfig {
    pub(super) fn namespace(mut self, exists: bool) -> Self {
        self.namespace_exists = exists;
        self
    }

    pub(super) fn ingress(mut self, value: Option<Value>) -> Self {
        self.ingress = value;
        self
    }

    pub(super) fn filter(mut self, value: Option<Value>) -> Self {
        self.filter = value;
        self
    }

    pub(super) fn client_secret(mut self, value: Option<Value>) -> Self {
        self.client_secret = value;
        self
    }

    pub(super) fn companion_secret(mut self, value: Option<Value>) -> Self {
        self.companion_secret = value;
        self
    }

    pub(super) fn retain_ingress_on_delete(mut self, retain: bool) -> Self {
        self.retain_ingress_on_delete = retain;
        self
    }
}

#[derive(Debug, Clone)]
pub(super) struct RecordedRequest {
    pub(super) method: String,
    pub(super) uri: String,
    pub(super) body: Option<Value>,
}

#[derive(Clone)]
pub(super) struct FakeKube {
    state: Arc<Mutex<FakeKubeState>>,
}

struct FakeKubeState {
    namespace: Option<Value>,
    ingress: Option<Value>,
    filter: Option<Value>,
    client_secret: Option<Value>,
    companion_secret: Option<Value>,
    namespace_path: String,
    ingress_path: String,
    filter_path: String,
    client_secret_path: Option<String>,
    companion_secret_path: Option<String>,
    retain_ingress_on_delete: bool,
    requests: Vec<RecordedRequest>,
}

impl FakeKube {
    pub(super) fn new(
        builder: &ResourceBuilder,
        workspace: &Workspace,
        config: FakeKubeConfig,
    ) -> Self {
        let names = builder.runtime_names(workspace).unwrap();
        let mtls = builder.ttyd_mtls.as_ref();
        Self {
            state: Arc::new(Mutex::new(FakeKubeState {
                namespace: config.namespace_exists.then(|| {
                    let mut labels = serde_json::Map::new();
                    labels.insert(
                        "workspace.memeloop.dev/owner-installation".to_owned(),
                        json!(builder.installation_id.as_str()),
                    );
                    json!({
                        "apiVersion": "v1",
                        "kind": "Namespace",
                        "metadata": {
                            "name": WORKSPACE_NAMESPACE,
                            "labels": labels,
                        },
                    })
                }),
                ingress: config.ingress,
                filter: config.filter,
                client_secret: config.client_secret,
                companion_secret: config.companion_secret,
                namespace_path: format!("/api/v1/namespaces/{WORKSPACE_NAMESPACE}"),
                ingress_path: ingress_path(&names.resources.web_shell_ingress),
                filter_path: filter_path(builder, &names.resources.web_shell_envoy_filter),
                client_secret_path: mtls.map(|_| client_secret_path(builder)),
                companion_secret_path: mtls.map(|_| companion_secret_path(builder)),
                retain_ingress_on_delete: config.retain_ingress_on_delete,
                requests: Vec::new(),
            })),
        }
    }

    pub(super) fn requests(&self) -> Vec<RecordedRequest> {
        self.state.lock().unwrap().requests.clone()
    }

    pub(super) fn set_ingress(&self, ingress: Option<Value>) {
        self.state.lock().unwrap().ingress = ingress;
    }

    pub(super) fn coordinator(&self, builder: ResourceBuilder) -> KubernetesCoordinator {
        let fake = self.clone();
        let service = service_fn(move |request: Request<KubeBody>| {
            let fake = fake.clone();
            async move { Ok::<_, Infallible>(fake.respond(request).await) }
        });
        KubernetesCoordinator::new(kube::Client::new(service, "default"), builder)
    }

    async fn respond(&self, request: Request<KubeBody>) -> Response<Body> {
        let (parts, body) = request.into_parts();
        let bytes = body.collect().await.unwrap().to_bytes();
        let body = (!bytes.is_empty())
            .then(|| serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null));
        let method = parts.method;
        let uri = parts.uri.to_string();
        let path = parts.uri.path().to_owned();
        let mut state = self.state.lock().unwrap();
        state.requests.push(RecordedRequest {
            method: method.to_string(),
            uri,
            body: body.clone(),
        });

        match method {
            Method::GET => state
                .get(&path)
                .map_or_else(not_found, |value| json_response(StatusCode::OK, value)),
            Method::PATCH => json_response(StatusCode::OK, body.unwrap_or_else(|| json!({}))),
            Method::DELETE => {
                if path == state.ingress_path && !state.retain_ingress_on_delete {
                    state.ingress = None;
                }
                if path == state.filter_path {
                    state.filter = None;
                }
                success()
            }
            _ => panic!("unexpected Kubernetes request: {method} {path}"),
        }
    }
}

impl FakeKubeState {
    fn get(&self, path: &str) -> Option<Value> {
        if path == self.namespace_path {
            return self.namespace.clone();
        }
        if path == self.ingress_path {
            return self.ingress.clone();
        }
        if path == self.filter_path {
            return self.filter.clone();
        }
        if self.client_secret_path.as_deref() == Some(path) {
            return self.client_secret.clone();
        }
        if self.companion_secret_path.as_deref() == Some(path) {
            return self.companion_secret.clone();
        }
        let kind = if path.ends_with("/pods") {
            Some("PodList")
        } else if path.ends_with("/ingresses") {
            Some("IngressList")
        } else if path.ends_with("/networkpolicies") {
            Some("NetworkPolicyList")
        } else if path.ends_with("/services") {
            Some("ServiceList")
        } else {
            None
        }?;
        Some(json!({
            "apiVersion": "v1",
            "kind": kind,
            "metadata": {"resourceVersion": "1"},
            "items": [],
        }))
    }
}

fn json_response(status: StatusCode, value: Value) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(value.to_string()))
        .unwrap()
}

fn not_found() -> Response<Body> {
    json_response(
        StatusCode::NOT_FOUND,
        json!({
            "apiVersion": "v1",
            "kind": "Status",
            "status": "Failure",
            "reason": "NotFound",
            "message": "not found",
            "code": 404,
        }),
    )
}

fn success() -> Response<Body> {
    json_response(
        StatusCode::OK,
        json!({
            "apiVersion": "v1",
            "kind": "Status",
            "status": "Success",
            "code": 200,
        }),
    )
}
