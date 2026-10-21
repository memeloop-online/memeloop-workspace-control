use super::*;
use http_body_util::BodyExt;
use serde_json::{Value, json};

const NAMESPACE: &str = "memeloop-workspace-control";
const MAPPING_LABEL: &str = "workspace.memeloop.dev/port-mapping-id";

#[derive(Default)]
struct DeleteMock {
    objects: Mutex<BTreeMap<String, Value>>,
    requests: Mutex<Vec<(Method, String, Value)>>,
    replace_before_delete: Mutex<Option<String>>,
}

impl DeleteMock {
    fn insert(&self, collection: &str, name: &str, kind: &str, workspace_id: Uuid) -> String {
        let path = format!("{collection}/{name}");
        self.objects.lock().unwrap().insert(
            path.clone(),
            json!({
                "apiVersion": if collection.starts_with("/api/") { "v1".to_owned() } else {
                    collection.split('/').skip(2).take(2).collect::<Vec<_>>().join("/")
                },
                "kind": kind,
                "metadata": {
                    "name": name, "namespace": collection.split("/namespaces/").nth(1)
                        .and_then(|part| part.split('/').next()),
                    "uid": format!("{name}-uid"),
                    "labels": ownership_labels("public-a", workspace_id)
                }
            }),
        );
        path
    }

    fn mapping(&self, collection: &str, name: &str, kind: &str, workspace_id: Uuid) -> String {
        let path = self.insert(collection, name, kind, workspace_id);
        self.objects.lock().unwrap().get_mut(&path).unwrap()["metadata"]["labels"][MAPPING_LABEL] =
            json!(name);
        path
    }

    fn terminating(&self, path: &str) {
        self.objects.lock().unwrap().get_mut(path).unwrap()["metadata"]["deletionTimestamp"] =
            json!("2026-10-21T00:00:00Z");
    }

    fn remove(&self, path: &str) {
        self.objects.lock().unwrap().remove(path);
    }

    fn exists(&self, path: &str) -> bool {
        self.objects.lock().unwrap().contains_key(path)
    }

    fn deletes(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(method, _, _)| method == Method::DELETE)
            .map(|(_, path, _)| path.clone())
            .collect()
    }

    async fn respond(&self, request: Request<kube::client::Body>) -> Response<Body> {
        let (parts, body) = request.into_parts();
        let bytes = body.collect().await.unwrap().to_bytes();
        let body: Value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        let path = parts.uri.path().to_owned();
        self.requests
            .lock()
            .unwrap()
            .push((parts.method.clone(), path.clone(), body.clone()));
        let mut objects = self.objects.lock().unwrap();
        if parts.method == Method::DELETE {
            if self.replace_before_delete.lock().unwrap().take().as_ref() == Some(&path) {
                objects.get_mut(&path).unwrap()["metadata"]["uid"] = json!("replacement-uid");
            }
            let Some(existing) = objects.get(&path) else {
                return not_found();
            };
            if body["preconditions"]["uid"] != existing["metadata"]["uid"] {
                return json_response(
                    StatusCode::CONFLICT,
                    json!({
                        "apiVersion":"v1", "kind":"Status", "status":"Failure",
                        "reason":"Conflict", "message":"UID precondition failed", "code":409
                    }),
                );
            }
            objects.remove(&path);
            return success();
        }
        assert_eq!(parts.method, Method::GET);
        if let Some(object) = objects.get(&path) {
            return json_response(StatusCode::OK, object.clone());
        }
        if [
            "/pods",
            "/services",
            "/ingresses",
            "/networkpolicies",
            "/envoyfilters",
        ]
        .iter()
        .any(|suffix| path.ends_with(suffix))
        {
            let mapping_only = parts.uri.query().is_some_and(|query| {
                url::form_urlencoded::parse(query.as_bytes())
                    .any(|(key, value)| key == "labelSelector" && value.contains(MAPPING_LABEL))
            });
            let prefix = format!("{path}/");
            let items: Vec<_> = objects
                .iter()
                .filter(|(key, value)| {
                    key.starts_with(&prefix)
                        && !key[prefix.len()..].contains('/')
                        && (!mapping_only
                            || value["metadata"]["labels"].get(MAPPING_LABEL).is_some())
                })
                .map(|(_, value)| value.clone())
                .collect();
            return json_response(
                StatusCode::OK,
                json!({
                    "apiVersion":"v1", "kind":"List", "metadata":{"resourceVersion":"1"}, "items":items
                }),
            );
        }
        not_found()
    }
}

fn setup() -> (
    Workspace,
    ResourceBuilder,
    Arc<DeleteMock>,
    KubernetesCoordinator,
) {
    let workspace = workspace(workspace_id());
    let mut builder =
        workspace_coordinator(Arc::new(WorkspaceDeleteMock::new("public-a", workspace.id))).builder;
    builder.port_mapping_domain = Some("ports.example.invalid".to_owned());
    builder.ttyd_mtls = Some(
        crate::kubernetes::TtydMtlsConfig::new(
            "server-tls".to_owned(),
            "client-ca".to_owned(),
            "higress-system".to_owned(),
            "higress-client".to_owned(),
        )
        .unwrap(),
    );
    let mock = Arc::new(DeleteMock::default());
    let namespace_path = format!("/api/v1/namespaces/{NAMESPACE}");
    mock.objects.lock().unwrap().insert(
        namespace_path,
        json!({
            "apiVersion":"v1", "kind":"Namespace", "metadata": {
                "name":NAMESPACE, "labels":{"workspace.memeloop.dev/owner-installation":"public-a"}
            }
        }),
    );
    let service = {
        let mock = mock.clone();
        service_fn(move |request| {
            let mock = mock.clone();
            async move { Ok::<_, Infallible>(mock.respond(request).await) }
        })
    };
    let coordinator =
        KubernetesCoordinator::new(kube::Client::new(service, "default"), builder.clone());
    (workspace, builder, mock, coordinator)
}

fn core(collection: &str) -> String {
    format!("/api/v1/namespaces/{NAMESPACE}/{collection}")
}

fn networking(collection: &str) -> String {
    format!("/apis/networking.k8s.io/v1/namespaces/{NAMESPACE}/{collection}")
}

#[tokio::test]
async fn all_inputs_delete_in_one_round_but_home_waits_for_the_next_round() {
    let (workspace, builder, mock, coordinator) = setup();
    let names = builder.runtime_names(&workspace).unwrap().resources;
    let mut inputs = Vec::new();
    for name in [
        &names.workspace_config,
        &names.http_proxy_config,
        &names.environment_config_map,
        &names.files_config_map,
        &names.startup_gate_config_map(),
    ] {
        inputs.push(mock.insert(&core("configmaps"), name, "ConfigMap", workspace.id));
    }
    for name in [
        &names.ssh_identity_secret,
        &names.environment_secret,
        &names.files_secret,
    ] {
        inputs.push(mock.insert(&core("secrets"), name, "Secret", workspace.id));
    }
    inputs.push(mock.insert(
        &networking("networkpolicies"),
        &names.network_policy,
        "NetworkPolicy",
        workspace.id,
    ));
    inputs.push(mock.insert(
        &core("serviceaccounts"),
        &names.service_account,
        "ServiceAccount",
        workspace.id,
    ));
    let home = mock.insert(
        &core("persistentvolumeclaims"),
        &names.data_pvc_ordinal_zero(),
        "PersistentVolumeClaim",
        workspace.id,
    );

    assert_eq!(
        coordinator.delete_or_confirm(&workspace).await.unwrap(),
        DeleteProgress::DeletionRequested
    );
    let mut deletes = mock.deletes();
    deletes.sort();
    inputs.sort();
    assert_eq!(deletes, inputs);
    assert!(mock.exists(&home));
    assert_eq!(
        coordinator.delete_or_confirm(&workspace).await.unwrap(),
        DeleteProgress::DeletionRequested
    );
    assert!(!mock.exists(&home));
    assert_eq!(
        coordinator.delete_or_confirm(&workspace).await.unwrap(),
        DeleteProgress::Gone
    );
    assert_eq!(mock.deletes().len(), inputs.len() + 1);
}

#[tokio::test]
async fn routes_filters_workload_and_inputs_keep_their_safety_boundaries() {
    let (workspace, builder, mock, coordinator) = setup();
    let names = builder.runtime_names(&workspace).unwrap().resources;
    let ingress = mock.insert(
        &networking("ingresses"),
        &names.web_shell_ingress,
        "Ingress",
        workspace.id,
    );
    mock.terminating(&ingress);
    let mapping_ingresses = ["mapping-a", "mapping-b"]
        .map(|name| mock.mapping(&networking("ingresses"), name, "Ingress", workspace.id));
    let filters = "/apis/networking.istio.io/v1alpha3/namespaces/higress-system/envoyfilters";
    let fixed_filter = mock.insert(
        filters,
        &names.web_shell_envoy_filter,
        "EnvoyFilter",
        workspace.id,
    );
    mock.terminating(&fixed_filter);
    let mut external = Vec::new();
    for name in ["mapping-a", "mapping-b"] {
        external.push(mock.mapping(filters, name, "EnvoyFilter", workspace.id));
        external.push(mock.mapping(
            &networking("networkpolicies"),
            name,
            "NetworkPolicy",
            workspace.id,
        ));
        external.push(mock.mapping(&core("services"), name, "Service", workspace.id));
    }
    external.push(mock.insert(&core("services"), &names.service, "Service", workspace.id));
    external.push(mock.insert(
        &core("services"),
        &names.ssh_service,
        "Service",
        workspace.id,
    ));
    let workload = mock.insert(
        &format!("/apis/apps/v1/namespaces/{NAMESPACE}/statefulsets"),
        &names.stateful_set,
        "StatefulSet",
        workspace.id,
    );
    mock.terminating(&workload);
    let pod = mock.insert(
        &core("pods"),
        &names.pod_ordinal_zero(),
        "Pod",
        workspace.id,
    );
    let input = mock.insert(
        &core("secrets"),
        &names.files_secret,
        "Secret",
        workspace.id,
    );

    assert_eq!(
        coordinator.delete_or_confirm(&workspace).await.unwrap(),
        DeleteProgress::DeletionRequested
    );
    assert_eq!(mock.deletes(), mapping_ingresses);
    assert!(mock.exists(&fixed_filter));
    assert!(external.iter().all(|path| mock.exists(path)));
    assert_eq!(
        coordinator.delete_or_confirm(&workspace).await.unwrap(),
        DeleteProgress::DeletionRequested
    );
    assert_eq!(
        mock.deletes().len(),
        2,
        "terminating ingress is not deleted again"
    );

    mock.remove(&ingress);
    assert_eq!(
        coordinator.delete_or_confirm(&workspace).await.unwrap(),
        DeleteProgress::DeletionRequested
    );
    assert!(external.iter().all(|path| !mock.exists(path)));
    assert!(mock.exists(&fixed_filter));
    assert!(mock.exists(&workload));
    assert!(mock.exists(&input));
    let delete_count = mock.deletes().len();
    assert_eq!(
        coordinator.delete_or_confirm(&workspace).await.unwrap(),
        DeleteProgress::DeletionRequested
    );
    assert_eq!(
        mock.deletes().len(),
        delete_count,
        "terminating filter remains a phase barrier"
    );

    mock.remove(&fixed_filter);
    assert_eq!(
        coordinator.delete_or_confirm(&workspace).await.unwrap(),
        DeleteProgress::DeletionRequested
    );
    assert_eq!(
        mock.deletes().len(),
        delete_count,
        "terminating StatefulSet is not deleted again"
    );
    assert!(mock.exists(&input));
    mock.remove(&workload);
    assert_eq!(
        coordinator.delete_or_confirm(&workspace).await.unwrap(),
        DeleteProgress::Terminating
    );
    assert!(mock.exists(&input));
    mock.remove(&pod);
    assert_eq!(
        coordinator.delete_or_confirm(&workspace).await.unwrap(),
        DeleteProgress::DeletionRequested
    );
    assert!(!mock.exists(&input));
}

#[tokio::test]
async fn fixed_and_all_mapping_filters_delete_together() {
    let (workspace, builder, mock, coordinator) = setup();
    let names = builder.runtime_names(&workspace).unwrap().resources;
    let filters = "/apis/networking.istio.io/v1alpha3/namespaces/higress-system/envoyfilters";
    let mut expected = vec![mock.insert(
        filters,
        &names.web_shell_envoy_filter,
        "EnvoyFilter",
        workspace.id,
    )];
    for name in ["mapping-a", "mapping-b"] {
        expected.push(mock.mapping(filters, name, "EnvoyFilter", workspace.id));
    }
    assert_eq!(
        coordinator.delete_or_confirm(&workspace).await.unwrap(),
        DeleteProgress::DeletionRequested
    );
    let mut deletes = mock.deletes();
    deletes.sort();
    expected.sort();
    assert_eq!(deletes, expected);
    assert_eq!(
        coordinator.delete_or_confirm(&workspace).await.unwrap(),
        DeleteProgress::Gone
    );
}

#[tokio::test]
async fn terminating_inputs_still_block_home_and_foreign_inputs_are_never_deleted() {
    for foreign in [false, true] {
        let (workspace, builder, mock, coordinator) = setup();
        let names = builder.runtime_names(&workspace).unwrap().resources;
        let first = mock.insert(
            &core("configmaps"),
            &names.workspace_config,
            "ConfigMap",
            workspace.id,
        );
        mock.terminating(&first);
        if foreign {
            mock.objects.lock().unwrap().get_mut(&first).unwrap()["metadata"]["labels"] =
                ownership_labels("other-installation", workspace.id);
        }
        let second = mock.insert(
            &core("secrets"),
            &names.files_secret,
            "Secret",
            workspace.id,
        );
        let home = mock.insert(
            &core("persistentvolumeclaims"),
            &names.data_pvc_ordinal_zero(),
            "PersistentVolumeClaim",
            workspace.id,
        );
        let result = coordinator.delete_or_confirm(&workspace).await;
        if foreign {
            assert!(matches!(result, Err(ReconcileError::Ownership(_))));
            assert!(mock.deletes().is_empty());
        } else {
            assert_eq!(result.unwrap(), DeleteProgress::DeletionRequested);
            assert_eq!(mock.deletes(), [second]);
        }
        assert!(mock.exists(&first));
        assert!(mock.exists(&home));
    }
}

#[tokio::test]
async fn uid_replacement_conflict_preserves_the_replacement_and_blocks_home() {
    let (workspace, builder, mock, coordinator) = setup();
    let names = builder.runtime_names(&workspace).unwrap().resources;
    let input = mock.insert(
        &core("configmaps"),
        &names.workspace_config,
        "ConfigMap",
        workspace.id,
    );
    let home = mock.insert(
        &core("persistentvolumeclaims"),
        &names.data_pvc_ordinal_zero(),
        "PersistentVolumeClaim",
        workspace.id,
    );
    *mock.replace_before_delete.lock().unwrap() = Some(input.clone());
    assert!(matches!(coordinator.delete_or_confirm(&workspace).await,
        Err(ReconcileError::Kubernetes(kube::Error::Api(error))) if error.code == 409));
    assert_eq!(
        mock.objects.lock().unwrap()[&input]["metadata"]["uid"],
        "replacement-uid"
    );
    assert!(mock.exists(&home));
}

#[tokio::test]
async fn missing_uid_never_produces_an_unguarded_delete() {
    let (workspace, builder, mock, coordinator) = setup();
    let names = builder.runtime_names(&workspace).unwrap().resources;
    let input = mock.insert(
        &core("configmaps"),
        &names.workspace_config,
        "ConfigMap",
        workspace.id,
    );
    mock.objects.lock().unwrap().get_mut(&input).unwrap()["metadata"]
        .as_object_mut()
        .unwrap()
        .remove("uid");
    assert!(matches!(
        coordinator.delete_or_confirm(&workspace).await,
        Err(ReconcileError::Kubernetes(kube::Error::Service(_)))
    ));
    assert!(mock.deletes().is_empty());
}

#[tokio::test]
async fn cluster_role_binding_stays_a_separate_first_phase() {
    let (workspace, builder, mock, coordinator) = setup();
    let names = builder.runtime_names(&workspace).unwrap().resources;
    let binding = mock.insert(
        "/apis/rbac.authorization.k8s.io/v1/clusterrolebindings",
        &builder.cluster_admin_binding_name(&workspace).unwrap(),
        "ClusterRoleBinding",
        workspace.id,
    );
    let ingress = mock.insert(
        &networking("ingresses"),
        &names.web_shell_ingress,
        "Ingress",
        workspace.id,
    );
    assert_eq!(
        coordinator.delete_or_confirm(&workspace).await.unwrap(),
        DeleteProgress::DeletionRequested
    );
    assert_eq!(mock.deletes(), [binding]);
    assert!(mock.exists(&ingress));
}
