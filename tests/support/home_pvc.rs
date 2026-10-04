use std::{
    convert::Infallible,
    net::SocketAddr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Router,
    body::Body,
    http::{Request, Response, StatusCode},
};
use http_body_util::BodyExt;
use memeloop_workspace_control::{
    api::{AppState, router},
    auth::ApiKeyScope,
    config::AppConfig,
    quota::Resources,
    storage::{CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate, Database},
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::{
        AccessMode, Workspace, WorkspaceAction, WorkspaceHomeVolumeBinding, WorkspaceObservation,
    },
};
use serde_json::{Value, json};
use tower::{ServiceExt, service_fn};

pub const ADMIN: &str = "home-binding-admin-000000000000000000000000000000";
pub const MEMBER: &str = "home-binding-member-00000000000000000000000000000";
const IMAGE: &str = "registry.example/workspace@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const NAMESPACE: &str = "memeloop-workspace-control";

pub struct Fixture {
    pub database: Database,
    pub workspace: Workspace,
    pub now: i64,
}

impl Fixture {
    pub async fn new() -> Self {
        let database = Database::connect("sqlite::memory:", "home-binding".parse().unwrap())
            .await
            .unwrap();
        Self::with_database(database).await
    }

    pub async fn with_database(database: Database) -> Self {
        database.migrate().await.unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        database
            .upsert_image_policy(IMAGE, true, now)
            .await
            .unwrap();
        let admin = database
            .create_user_with_initial_key(
                "Admin",
                ADMIN,
                true,
                ApiKeyScope::initial_key_defaults(true),
                now + 3600,
                now,
            )
            .await
            .unwrap();
        database
            .create_user_with_initial_key(
                "Member",
                MEMBER,
                false,
                ApiKeyScope::initial_key_defaults(false),
                now + 3600,
                now,
            )
            .await
            .unwrap();
        let organization = database
            .create_organization(
                CreateOrganization {
                    name: "Home binding".to_owned(),
                    owner_user_id: admin.user_id,
                },
                now,
            )
            .await
            .unwrap();
        let yaml = WorkspaceTemplateDocument::new(
            "Home binding",
            WorkspaceTemplateSpec::standard(
                IMAGE,
                AccessMode::Internal,
                Resources {
                    cpu_millis: 1000,
                    memory_mib: 1024,
                    gpu_count: 0,
                    disk_gib: 100,
                },
            ),
        )
        .to_yaml()
        .unwrap();
        let template = database
            .create_workspace_template(
                CreateWorkspaceTemplate {
                    organization_id: Some(organization.id),
                    yaml,
                },
                true,
                now,
            )
            .await
            .unwrap();
        let workspace = database
            .create_workspace(
                CreateWorkspace {
                    organization_id: organization.id,
                    owner_id: admin.user_id,
                    name: "Home binding".to_owned(),
                    template_id: template.id,
                    resources: None,
                    organization_injection_refs: None,
                    user_injection_refs: None,
                },
                true,
                admin.user_id,
                now,
            )
            .await
            .unwrap();
        database
            .request_workspace_action(workspace.id, WorkspaceAction::Stop, admin.user_id, now)
            .await
            .unwrap();
        let workspace = database
            .record_workspace_observation(
                workspace.id,
                WorkspaceObservation::Stopped,
                admin.user_id,
                now,
            )
            .await
            .unwrap();
        Self {
            database,
            workspace,
            now,
        }
    }

    pub fn binding(&self) -> WorkspaceHomeVolumeBinding {
        WorkspaceHomeVolumeBinding {
            namespace: NAMESPACE.to_owned(),
            claim_name: "migrated-home-20gi".to_owned(),
            claim_uid: "pvc-original-uid".to_owned(),
            capacity_gib: 20,
        }
    }

    pub fn input(&self) -> Value {
        let mut input = serde_json::to_value(self.binding()).unwrap();
        input["expected_generation"] = self.workspace.generation.into();
        input
    }

    pub fn claim(&self) -> Value {
        json!({"apiVersion":"v1","kind":"PersistentVolumeClaim","metadata":{"name":self.binding().claim_name,"namespace":NAMESPACE,"uid":self.binding().claim_uid},"spec":{"accessModes":["ReadWriteOnce"],"volumeName":"pv-home","resources":{"requests":{"storage":"20Gi"}}},"status":{"phase":"Bound","capacity":{"storage":"20Gi"}}})
    }

    pub fn app(&self, claim: Value, pods: Vec<Value>, workloads: Vec<Value>) -> Router {
        let service = service_fn(move |request: Request<kube::client::Body>| {
            let claim = claim.clone();
            let pods = pods.clone();
            let workloads = workloads.clone();
            async move {
                assert_eq!(
                    request.method(),
                    "GET",
                    "binding must never mutate Kubernetes"
                );
                let path = request.uri().path();
                let (status, body) = if path.ends_with("/persistentvolumeclaims/migrated-home-20gi")
                    && !claim.is_null()
                {
                    (StatusCode::OK, claim)
                } else if path.ends_with("/persistentvolumeclaims") {
                    (
                        StatusCode::OK,
                        json!({"apiVersion":"v1","kind":"PersistentVolumeClaimList","metadata":{},"items":[]}),
                    )
                } else if path.starts_with("/api/v1/") && path.ends_with("/pods") {
                    (
                        StatusCode::OK,
                        json!({"apiVersion":"v1","kind":"PodList","metadata":{},"items":pods}),
                    )
                } else if path.ends_with("/statefulsets") {
                    (
                        StatusCode::OK,
                        json!({"apiVersion":"apps/v1","kind":"StatefulSetList","metadata":{},"items":workloads}),
                    )
                } else if path.ends_with("/events") {
                    (
                        StatusCode::OK,
                        json!({"apiVersion":"v1","kind":"EventList","metadata":{},"items":[]}),
                    )
                } else {
                    (
                        StatusCode::NOT_FOUND,
                        json!({"apiVersion":"v1","kind":"Status","status":"Failure","reason":"NotFound","message":"fixture missing","code":404}),
                    )
                };
                Ok::<_, Infallible>(
                    Response::builder()
                        .status(status)
                        .header("Content-Type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
            }
        });
        let mut state = self.state();
        state.set_kubernetes_client(kube::Client::new(service, NAMESPACE));
        router(Arc::new(state))
    }

    pub fn state(&self) -> AppState {
        let installation_id = match &self.database {
            Database::Sqlite {
                installation_id, ..
            }
            | Database::Postgres {
                installation_id, ..
            } => installation_id.clone(),
        };
        AppState::new(
            AppConfig {
                installation_id,
                listen_address: SocketAddr::from(([127, 0, 0, 1], 0)),
                database_url: "sqlite::memory:".to_owned(),
                replica_count: 1,
                instance_id: "home-test".to_owned(),
                ssh_public_host: None,
                internal_ssh_host: None,
                web_shell_public_origin: None,
                port_mapping_public_domain: None,
                prometheus_url: None,
                plugin_dir: None,
            },
            self.database.clone(),
        )
    }

    pub async fn ready(&mut self) {
        self.database
            .request_workspace_action(
                self.workspace.id,
                WorkspaceAction::Start,
                self.workspace.owner_id,
                self.now,
            )
            .await
            .unwrap();
        self.workspace = self
            .database
            .record_workspace_observation(
                self.workspace.id,
                WorkspaceObservation::Ready,
                self.workspace.owner_id,
                self.now,
            )
            .await
            .unwrap();
        match &self.database {
            Database::Sqlite { pool, .. } => {
                sqlx::query("UPDATE jobs SET status = 'completed' WHERE workspace_id = ?1")
                    .bind(self.workspace.id.to_string())
                    .execute(pool)
                    .await
                    .unwrap();
            }
            Database::Postgres { pool, .. } => {
                sqlx::query("UPDATE jobs SET status = 'completed' WHERE workspace_id = $1")
                    .bind(self.workspace.id.to_string())
                    .execute(pool)
                    .await
                    .unwrap();
            }
        }
    }

    pub async fn terminal_failure(&mut self) {
        assert!(
            self.database
                .mark_workspace_failed_if_generation(
                    self.workspace.id,
                    self.workspace.generation,
                    self.now,
                )
                .await
                .unwrap()
        );
        self.workspace = self
            .database
            .get_workspace(self.workspace.id)
            .await
            .unwrap();
    }

    pub fn adopted_runtime(&self) -> (Value, Value) {
        let name = format!("w-{}", self.workspace.short_id);
        let labels = json!({"workspace.memeloop.dev/workspace-id":self.workspace.id,"workspace.memeloop.dev/owner-installation":"home-binding"});
        let spec = json!({"containers":[{"name":"workspace","image":IMAGE,"volumeMounts":[{"name":"workspace-data","mountPath":self.workspace.template.workspace_home}]}],"volumes":[{"name":"workspace-data","persistentVolumeClaim":{"claimName":self.binding().claim_name}}]});
        let sts = json!({"apiVersion":"apps/v1","kind":"StatefulSet","metadata":{"name":name,"namespace":NAMESPACE,"uid":"sts-uid","labels":labels},"spec":{"replicas":1,"serviceName":name,"selector":{"matchLabels":labels},"template":{"metadata":{"labels":labels},"spec":spec}},"status":{"readyReplicas":1}});
        let pod = json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":format!("{name}-0"),"namespace":NAMESPACE,"uid":"pod-uid","labels":labels,"ownerReferences":[{"apiVersion":"apps/v1","kind":"StatefulSet","name":name,"uid":"sts-uid","controller":true}]},"spec":spec,"status":{"phase":"Running","conditions":[{"type":"Ready","status":"True"}]}});
        (pod, sts)
    }
}

pub async fn put(
    app: Router,
    workspace: &Workspace,
    token: &str,
    input: Value,
    key: &str,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/workspaces/{}/home-pvc", workspace.id))
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .header("Idempotency-Key", key)
        .body(Body::from(input.to_string()))
        .unwrap();
    decode(app.oneshot(request).await.unwrap()).await
}

pub async fn decode(response: axum::response::Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}
