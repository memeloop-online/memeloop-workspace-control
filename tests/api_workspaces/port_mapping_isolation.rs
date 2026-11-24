use std::{collections::BTreeMap, convert::Infallible, sync::Mutex, time::Duration};

use axum::response::IntoResponse;
use memeloop_workspace_control::{
    jobs::{ControlPlaneJobHandler, JobWorker, WebhookDeliveryHandler, WorkspaceReconcileHandler},
    kubernetes::{KubernetesCoordinator, ResourceBuilder},
    storage::{CreateOrganization, CreateWorkspace},
    workspace_runtime::WorkspaceRuntimeNames,
    workspaces::{Workspace, WorkspaceState},
};
use tower::service_fn;

use super::*;

const DOMAIN: &str = "apps.example.test";
const LIVE_IMAGE: &str = "registry.example/workspace:older-running";

struct KubernetesFixture {
    namespace: String,
    workspace_config: String,
    pod: String,
    objects: Mutex<BTreeMap<String, Value>>,
}

impl KubernetesFixture {
    fn new(workspace: &Workspace, database: &Database) -> Self {
        let names = WorkspaceRuntimeNames::for_workspace(
            database.installation_id(),
            &workspace.runtime,
            &workspace.short_id,
        )
        .unwrap();
        let labels = json!({
            "workspace.memeloop.dev/owner-installation": database.installation_id().as_str(),
            "workspace.memeloop.dev/workspace-id": workspace.id,
        });
        let pod = names.resources.pod_ordinal_zero();
        let workspace_config = names.resources.workspace_config;
        assert_ne!(workspace.template.image, LIVE_IMAGE);
        let objects = BTreeMap::from([
            (
                format!(
                    "/api/v1/namespaces/{}/configmaps/{workspace_config}",
                    names.namespace
                ),
                json!({
                    "apiVersion":"v1","kind":"ConfigMap",
                    "metadata":{"name":workspace_config,"labels":labels},
                    "data":{"sshd_config":"unchanged sshd","mwc-workspace-bootstrap":"unchanged bootstrap"}
                }),
            ),
            (
                format!("/api/v1/namespaces/{}/pods/{pod}", names.namespace),
                json!({
                    "apiVersion":"v1","kind":"Pod",
                    "metadata":{"name":pod,"labels":labels,"uid":"original-pod","resourceVersion":"1"},
                    "spec":{"containers":[{"name":"workspace","image":LIVE_IMAGE}]}
                }),
            ),
        ]);
        Self {
            namespace: names.namespace,
            workspace_config,
            pod,
            objects: Mutex::new(objects),
        }
    }

    fn client(self: &Arc<Self>) -> kube::Client {
        let fixture = self.clone();
        kube::Client::new(
            service_fn(move |request| {
                let fixture = fixture.clone();
                async move { Ok::<_, Infallible>(fixture.respond(request).await) }
            }),
            &self.namespace,
        )
    }

    async fn respond(&self, request: Request<kube::client::Body>) -> axum::response::Response {
        let path = request.uri().path().to_owned();
        let method = request.method().clone();
        let payload = request.into_body().collect().await.unwrap().to_bytes();
        let mut objects = self.objects.lock().unwrap();
        let path_items = path.split('/').collect::<Vec<_>>();
        let (api, kind, resource) = match path_items.as_slice() {
            ["", "api", "v1", "namespaces", _, "services", rest @ ..] => ("v1", "Service", rest),
            [
                "",
                "apis",
                "networking.k8s.io",
                "v1",
                "namespaces",
                _,
                "ingresses",
                rest @ ..,
            ] => ("networking.k8s.io/v1", "Ingress", rest),
            [
                "",
                "apis",
                "networking.k8s.io",
                "v1",
                "namespaces",
                _,
                "networkpolicies",
                rest @ ..,
            ] => ("networking.k8s.io/v1", "NetworkPolicy", rest),
            ["", "api", "v1", "namespaces", _, "pods", name] if *name == self.pod => {
                ("v1", "Pod", &path_items[path_items.len() - 1..])
            }
            ["", "api", "v1", "namespaces", _, "configmaps", name]
                if *name == self.workspace_config =>
            {
                ("v1", "ConfigMap", &path_items[path_items.len() - 1..])
            }
            _ => panic!("port mapping must not access workload resources: {method} {path}"),
        };
        if let Some(name) = resource.first() {
            assert!(
                matches!(kind, "Pod" | "ConfigMap") || name.starts_with("port-"),
                "port mapping must not change the workspace's base network: {path}"
            );
        }
        let response = if method == Method::GET && resource.is_empty() {
            let items = objects
                .iter()
                .filter_map(|(key, value)| {
                    key.strip_prefix(path.as_str())
                        .is_some_and(|tail| tail.starts_with("/port-"))
                        .then_some(value)
                })
                .collect::<Vec<_>>();
            json!({
                "apiVersion":api,"kind":format!("{kind}List"),
                "metadata":{"resourceVersion":"1"},"items":items
            })
        } else if method == Method::GET {
            let Some(object) = objects.get(&path) else {
                return (
                    StatusCode::NOT_FOUND,
                    axum::Json(json!({
                        "apiVersion":"v1","kind":"Status","code":404,
                        "reason":"NotFound","message":"fixture object not found"
                    })),
                )
                    .into_response();
            };
            object.clone()
        } else if method == Method::PATCH {
            let patch: Value = serde_json::from_slice(&payload).unwrap();
            match kind {
                "Pod" => {
                    assert_eq!(patch.as_object().unwrap().len(), 1);
                    assert_eq!(patch["metadata"]["uid"], "original-pod");
                    assert!(
                        patch["metadata"]["annotations"]
                            .get("workspace.memeloop.dev/port-mappings-hash")
                            .is_some()
                    );
                    let object = objects.get_mut(&path).unwrap();
                    object["metadata"]["annotations"] = patch["metadata"]["annotations"].clone();
                    object.clone()
                }
                "ConfigMap" => {
                    assert_eq!(patch["data"].as_object().unwrap().len(), 1);
                    let object = objects.get_mut(&path).unwrap();
                    object["data"]["port-mappings.json"] =
                        patch["data"]["port-mappings.json"].clone();
                    object.clone()
                }
                _ => {
                    objects.insert(path, patch.clone());
                    patch
                }
            }
        } else {
            assert_eq!(method, Method::DELETE);
            assert!(!matches!(kind, "Pod" | "ConfigMap"));
            assert!(objects.remove(&path).is_some());
            json!({"apiVersion":"v1","kind":"Status","status":"Success","code":200})
        };
        axum::Json(response).into_response()
    }

    fn check(&self, mapping: Uuid, port: Option<u16>, name: Option<&str>) {
        let objects = self.objects.lock().unwrap();
        let pod = &objects[&format!("/api/v1/namespaces/{}/pods/{}", self.namespace, self.pod)];
        assert_eq!(pod["metadata"]["uid"], "original-pod");
        assert_eq!(pod["spec"]["containers"][0]["image"], LIVE_IMAGE);
        let config = &objects[&format!(
            "/api/v1/namespaces/{}/configmaps/{}",
            self.namespace, self.workspace_config
        )];
        assert_eq!(config["data"]["sshd_config"], "unchanged sshd");
        assert_eq!(
            config["data"]["mwc-workspace-bootstrap"],
            "unchanged bootstrap"
        );
        let projected: Value =
            serde_json::from_str(config["data"]["port-mappings.json"].as_str().unwrap()).unwrap();
        let routes: Vec<_> = objects
            .values()
            .filter(|obj| {
                matches!(
                    obj["kind"].as_str(),
                    Some("Ingress" | "Service" | "NetworkPolicy")
                )
            })
            .collect();
        if let Some(port) = port {
            assert_eq!(routes.len(), 3);
            let ingress = routes.iter().find(|obj| obj["kind"] == "Ingress").unwrap();
            assert_eq!(
                ingress["metadata"]["name"],
                format!("port-{}", mapping.simple())
            );
            assert_eq!(
                ingress["spec"]["rules"][0]["http"]["paths"][0]["backend"]["service"]["port"]["number"],
                port
            );
            assert_eq!(projected[0]["internal_port"], port);
            assert_eq!(projected[0]["display_name"], name.unwrap());
            assert_eq!(projected[0]["id"], mapping.to_string());
        } else {
            assert!(routes.is_empty());
            assert_eq!(projected, json!([]));
        }
    }
}

fn resource_builder(database: &Database) -> ResourceBuilder {
    ResourceBuilder {
        installation_id: database.installation_id().clone(),
        ttyd_image: "example/ttyd:1".into(),
        buildkit_image: "example/buildkit:1".into(),
        ttyd_mtls: None,
        higress_namespace: "higress-system".into(),
        higress_pod_labels: BTreeMap::from([("app".into(), "gateway".into())]),
        higress_source_cidrs: Vec::new(),
        internet_egress: None,
        jump_host_namespace: "access".into(),
        jump_host_pod_labels: BTreeMap::new(),
        storage_class_name: None,
        scratch_storage_class_name: "scratch".into(),
        web_shell_domain: None,
        port_mapping_domain: Some(DOMAIN.into()),
        higress_gateway_name: "higress".into(),
        higress_https_section_name: "https".into(),
        internal_ssh_node_port_enabled: false,
    }
}

async fn ready_workspace(database: &Database, owner: Uuid, now: i64) -> Workspace {
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Port routes".into(),
                owner_user_id: owner,
            },
            now,
        )
        .await
        .unwrap();
    let template = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(organization.id),
                yaml: WorkspaceTemplateDocument::new(
                    "Port routes",
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
            true,
            now,
        )
        .await
        .unwrap();
    let workspace = database
        .create_workspace(
            CreateWorkspace {
                organization_id: organization.id,
                owner_id: owner,
                name: "Workspace with desired image staged".into(),
                template_id: template.id,
                resources: None,
                temporary_storage_gib: None,
                use_longhorn: false,
                organization_injection_refs: None,
                user_injection_refs: None,
            },
            true,
            owner,
            now,
        )
        .await
        .unwrap();
    let job = database
        .claim_job("seed", now, Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    database.complete_job(job.id, "seed", now).await.unwrap();
    database
        .record_workspace_observation(
            workspace.id,
            workspace.generation,
            WorkspaceObservation::Ready,
            owner,
            now,
        )
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn editing_port_mappings_applies_routes_without_reconciling_staged_workload() {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let database = Database::connect("sqlite::memory:", "port-worker-api".parse().unwrap())
        .await
        .unwrap();
    let (_, owner) = test_state(database.clone(), "sqlite::memory:").await;
    let workspace = ready_workspace(&database, owner, now).await;
    let kube = Arc::new(KubernetesFixture::new(&workspace, &database));
    let builder = resource_builder(&database);
    let cipher =
        EnvelopeCipher::from_base64("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap();
    let handler = ControlPlaneJobHandler::new(
        Some(WorkspaceReconcileHandler::new(
            database.clone(),
            cipher.clone(),
            builder.clone(),
            KubernetesCoordinator::new(kube.client(), builder),
        )),
        WebhookDeliveryHandler::new(database.clone(), cipher, Default::default()).unwrap(),
    );
    let worker = JobWorker::new(database.clone(), Arc::new(handler), "route-worker".into());
    let mut state = AppState::new(
        AppConfig {
            installation_id: database.installation_id().clone(),
            listen_address: SocketAddr::from(([127, 0, 0, 1], 0)),
            database_url: "sqlite::memory:".into(),
            replica_count: 1,
            instance_id: "test".into(),
            ssh_public_host: None,
            internal_ssh_host: None,
            web_shell_public_origin: None,
            port_mapping_public_domain: Some(DOMAIN.into()),
            prometheus_url: None,
            plugin_dir: None,
        },
        database.clone(),
    );
    state.set_kubernetes_client(kube.client());
    let app = router(Arc::new(state));
    let uri = format!("/api/v1/workspaces/{}/port-mappings", workspace.id);
    let response = app
        .clone()
        .oneshot(request(
            Method::POST,
            &uri,
            Some(ADMIN_TOKEN),
            Some("port-add"),
            Some(json!({
                "internal_port":3000,"display_name":"Preview","subdomain":"preview"
            })),
        ))
        .await
        .unwrap();
    let (status, bytes) = body(response).await;
    assert_eq!(status, StatusCode::CREATED);
    let added: Value = serde_json::from_slice(&bytes).unwrap();
    let mapping: Uuid = serde_json::from_value(added["id"].clone()).unwrap();
    let mapping_uri = format!("{uri}/{mapping}");
    assert!(worker.run_once(now + 60).await.unwrap());
    kube.check(mapping, Some(3000), Some("Preview"));

    for (port, name) in [(3000, "Name only edit"), (3001, "New port")] {
        let response = app
            .clone()
            .oneshot(request(
                Method::PUT,
                &mapping_uri,
                Some(ADMIN_TOKEN),
                None,
                Some(json!({
                    "internal_port":port,"display_name":name
                })),
            ))
            .await
            .unwrap();
        let (status, bytes) = body(response).await;
        assert_eq!(status, StatusCode::OK);
        let edited: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(edited["id"], added["id"]);
        assert_eq!(edited["https_url"], added["https_url"]);
        assert!(worker.run_once(now + 60).await.unwrap());
        kube.check(mapping, Some(port), Some(name));
    }

    let response = app
        .oneshot(request(
            Method::DELETE,
            &mapping_uri,
            Some(ADMIN_TOKEN),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(worker.run_once(now + 60).await.unwrap());
    kube.check(mapping, None, None);
    assert!(!worker.run_once(now + 60).await.unwrap());
    let persisted = database.get_workspace(workspace.id).await.unwrap();
    assert_eq!(persisted.state, WorkspaceState::Ready);
    assert_eq!(persisted.generation, workspace.generation);
    assert_eq!(persisted.template.image, workspace.template.image);
}
