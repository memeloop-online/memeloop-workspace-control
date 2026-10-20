use std::{
    collections::{BTreeMap, VecDeque},
    convert::Infallible,
    sync::{Arc, Mutex},
};

use axum::{
    body::{Body, to_bytes},
    http::{Request, Response},
};
use k8s_openapi::api::apps::v1::{
    StatefulSetPersistentVolumeClaimRetentionPolicy, StatefulSetStatus,
};
use serde_json::{Value, json};
use tower::service_fn;

use crate::{kubernetes::ResourceBuilder, workspaces::Workspace};

use super::super::KubernetesCoordinator;

struct CapturedRequest {
    method: String,
    uri: String,
    content_type: Option<String>,
    payload: Value,
}

type CapturedRequests = Arc<Mutex<Vec<CapturedRequest>>>;

#[derive(Clone, Copy)]
enum WorkloadPatch {
    RetainSources,
    StopRetention,
    StopReplicas,
}

fn owned_fields(value: &Value, field: &str) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(name, value)| (format!("f:{name}"), owned_fields(value, name)))
                .collect(),
        ),
        Value::Array(items)
            if matches!(
                field,
                "containers" | "initContainers" | "env" | "ports" | "volumes" | "volumeMounts"
            ) =>
        {
            Value::Object(
                items
                    .iter()
                    .map(|item| {
                        let selector = match field {
                            "ports" => json!({
                                "containerPort": item["containerPort"],
                                "protocol": item.get("protocol").cloned().unwrap_or(json!("TCP"))
                            }),
                            "volumeMounts" => json!({"mountPath": item["mountPath"]}),
                            _ => json!({"name": item["name"]}),
                        };
                        let mut fields = owned_fields(item, "");
                        fields
                            .as_object_mut()
                            .unwrap()
                            .insert(".".to_owned(), json!({}));
                        (format!("k:{selector}"), fields)
                    })
                    .collect(),
            )
        }
        _ => json!({}),
    }
}

fn mock_coordinator(
    patch: WorkloadPatch,
) -> (KubernetesCoordinator, Workspace, CapturedRequests, Value) {
    let workspace = super::workspace();
    let builder = ResourceBuilder {
        installation_id: "shrink-field-manager".parse().unwrap(),
        ttyd_image: "example/ttyd:1".to_owned(),
        buildkit_image: "moby/buildkit:test".to_owned(),
        ttyd_mtls: None,
        higress_namespace: "higress-system".to_owned(),
        higress_pod_labels: BTreeMap::new(),
        higress_source_cidrs: Vec::new(),
        internet_egress: None,
        jump_host_namespace: "access".to_owned(),
        jump_host_pod_labels: BTreeMap::new(),
        storage_class_name: None,
        scratch_storage_class_name: "local-nvme-scratch".to_owned(),
        web_shell_domain: None,
        port_mapping_domain: None,
        higress_gateway_name: "higress".to_owned(),
        higress_https_section_name: "https".to_owned(),
        internal_ssh_node_port_enabled: false,
    };
    let mut stateful_set = builder.build(&workspace).unwrap().stateful_set;
    stateful_set.metadata.uid = Some("set-uid".to_owned());
    stateful_set.metadata.resource_version = Some("17".to_owned());
    stateful_set.metadata.generation = Some(1);
    stateful_set.status = Some(StatefulSetStatus {
        observed_generation: Some(1),
        ..Default::default()
    });
    stateful_set.spec.as_mut().unwrap().replicas = Some(3);
    stateful_set
        .spec
        .as_mut()
        .unwrap()
        .persistent_volume_claim_retention_policy = matches!(patch, WorkloadPatch::StopReplicas)
        .then(|| StatefulSetPersistentVolumeClaimRetentionPolicy {
            when_deleted: Some("Retain".to_owned()),
            when_scaled: Some("Retain".to_owned()),
        });
    for container in &mut stateful_set
        .spec
        .as_mut()
        .unwrap()
        .template
        .spec
        .as_mut()
        .unwrap()
        .containers
    {
        for port in container.ports.iter_mut().flatten() {
            if port.protocol.is_none() {
                port.protocol = Some("TCP".to_owned());
            }
        }
    }
    let source = serde_json::to_value(stateful_set).unwrap();
    let fields = json!({
        "f:metadata": {"f:labels": owned_fields(&source["metadata"]["labels"], "labels")},
        "f:spec": owned_fields(&source["spec"], "spec")
    });
    let mut response = source.clone();
    response["metadata"]["annotations"] = json!({"foreign.example/private": "do-not-copy"});
    response["spec"]["revisionHistoryLimit"] = json!(21);
    response["spec"]["template"]["spec"]["priorityClassName"] = json!("foreign-priority");
    let container = &mut response["spec"]["template"]["spec"]["containers"][0];
    if container.get("env").is_none() {
        container["env"] = json!([]);
    }
    container["env"].as_array_mut().unwrap().push(json!({
        "name": "FOREIGN_INJECTED", "value": "not-owned"
    }));
    let container_selector = format!("k:{}", json!({"name": container["name"]}));
    response["metadata"]["managedFields"] = json!([
        {
            "manager": "memeloop-workspace-control", "operation": "Apply",
            "apiVersion": "apps/v1", "fieldsType": "FieldsV1", "fieldsV1": fields
        },
        {
            "manager": "foreign-injector", "operation": "Update", "apiVersion": "apps/v1",
            "fieldsType": "FieldsV1", "fieldsV1": {
                "f:metadata": {"f:annotations": {"f:foreign.example/private": {}}},
                "f:spec": {
                    "f:revisionHistoryLimit": {},
                    "f:template": {"f:spec": {
                        "f:priorityClassName": {},
                        "f:containers": {(container_selector): {
                            "f:env": {"k:{\"name\":\"FOREIGN_INJECTED\"}": {
                                ".": {}, "f:name": {}, "f:value": {}
                            }}
                        }}
                    }}
                }
            }
        },
        {
            "manager": "kube-controller-manager", "operation": "Update",
            "apiVersion": "apps/v1", "subresource": "status", "fieldsType": "FieldsV1",
            "fieldsV1": {"f:status": {"f:observedGeneration": {}}}
        }
    ]);
    let replies = Arc::new(Mutex::new(VecDeque::from([response.clone(), response])));
    let requests = CapturedRequests::default();
    let captured = requests.clone();
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
            captured.lock().unwrap().push(CapturedRequest {
                method: parts.method.to_string(),
                uri: parts.uri.to_string(),
                content_type: parts
                    .headers
                    .get("content-type")
                    .map(|value| value.to_str().unwrap().to_owned()),
                payload,
            });
            let reply = replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected Kubernetes request");
            Ok::<_, Infallible>(
                Response::builder()
                    .status(200)
                    .header("content-type", "application/json")
                    .body(Body::from(reply.to_string()))
                    .unwrap(),
            )
        }
    });
    let client = kube::Client::new(service, crate::workspace_runtime::WORKSPACE_NAMESPACE);
    (
        KubernetesCoordinator::new(client, builder),
        workspace,
        requests,
        source,
    )
}

async fn assert_workload_patch(patch: WorkloadPatch) {
    let (coordinator, workspace, requests, source) = mock_coordinator(patch);
    let operation = super::operation(&workspace);
    let result = match patch {
        WorkloadPatch::RetainSources => {
            coordinator
                .retain_shrink_sources(&workspace, &operation)
                .await
        }
        WorkloadPatch::StopRetention | WorkloadPatch::StopReplicas => {
            coordinator
                .stop_for_storage_shrink(&workspace, &operation)
                .await
        }
    };
    assert!(!result.unwrap());
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "GET");
    let request = &requests[1];
    assert_eq!(request.method, "PATCH");
    assert_eq!(
        request.content_type.as_deref(),
        Some("application/apply-patch+yaml")
    );
    assert_eq!(
        request.uri.split('?').next(),
        requests[0].uri.split('?').next()
    );
    let query = request.uri.split_once('?').unwrap().1;
    assert!(
        query
            .split('&')
            .any(|parameter| { parameter == "fieldManager=memeloop-workspace-control" })
    );
    assert!(!query.split('&').any(|parameter| parameter == "force=true"));
    let mut spec = source["spec"].clone();
    spec["persistentVolumeClaimRetentionPolicy"] = json!({
        "whenScaled": "Retain", "whenDeleted": "Retain"
    });
    if matches!(patch, WorkloadPatch::StopReplicas) {
        spec["replicas"] = json!(0);
    }
    let payload = &request.payload;
    assert_eq!(payload["spec"]["template"], source["spec"]["template"]);
    assert_eq!(payload["spec"]["selector"], source["spec"]["selector"]);
    assert_eq!(
        payload["spec"]["serviceName"],
        source["spec"]["serviceName"]
    );
    assert_eq!(
        payload["spec"]["volumeClaimTemplates"],
        source["spec"]["volumeClaimTemplates"]
    );
    assert_eq!(
        payload,
        &json!({
            "apiVersion": "apps/v1", "kind": "StatefulSet",
            "metadata": {
                "name": source["metadata"]["name"],
                "namespace": source["metadata"]["namespace"],
                "uid": "set-uid", "resourceVersion": "17", "labels": source["metadata"]["labels"]
            },
            "spec": spec
        })
    );
}

#[tokio::test]
async fn retaining_sources_uses_workload_field_manager_with_identity_guards() {
    assert_workload_patch(WorkloadPatch::RetainSources).await;
}

#[tokio::test]
async fn stopping_retention_uses_workload_field_manager_with_identity_guards() {
    assert_workload_patch(WorkloadPatch::StopRetention).await;
}

#[tokio::test]
async fn stopping_replicas_uses_workload_field_manager_with_identity_guards() {
    assert_workload_patch(WorkloadPatch::StopReplicas).await;
}
