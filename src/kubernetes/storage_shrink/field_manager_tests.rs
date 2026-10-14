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

type CapturedRequests = Arc<Mutex<Vec<(String, String, Value)>>>;

#[derive(Clone, Copy)]
enum WorkloadPatch {
    RetainSources,
    StopRetention,
    StopReplicas,
}

fn mock_coordinator(patch: WorkloadPatch) -> (KubernetesCoordinator, Workspace, CapturedRequests) {
    let workspace = super::workspace();
    let builder = ResourceBuilder {
        installation_id: "shrink-field-manager-test".parse().unwrap(),
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
        scratch_storage_class_name: None,
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
    stateful_set
        .spec
        .as_mut()
        .unwrap()
        .persistent_volume_claim_retention_policy = matches!(patch, WorkloadPatch::StopReplicas)
        .then(|| StatefulSetPersistentVolumeClaimRetentionPolicy {
            when_deleted: Some("Retain".to_owned()),
            when_scaled: Some("Retain".to_owned()),
        });
    let response = serde_json::to_value(stateful_set).unwrap();
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
            captured.lock().unwrap().push((
                parts.method.to_string(),
                parts.uri.to_string(),
                payload,
            ));
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
    )
}

async fn assert_workload_patch(patch: WorkloadPatch) {
    let (coordinator, workspace, requests) = mock_coordinator(patch);
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
    assert_eq!(requests[0].0, "GET");
    let (method, uri, payload) = &requests[1];
    assert_eq!(method, "PATCH");
    let query = uri.split_once('?').unwrap().1;
    assert!(
        query
            .split('&')
            .any(|parameter| { parameter == "fieldManager=memeloop-workspace-control" })
    );
    assert!(!query.split('&').any(|parameter| parameter == "force=true"));
    let spec = match patch {
        WorkloadPatch::StopReplicas => json!({"replicas": 0}),
        WorkloadPatch::RetainSources | WorkloadPatch::StopRetention => {
            json!({"persistentVolumeClaimRetentionPolicy": {
                "whenScaled": "Retain", "whenDeleted": "Retain"
            }})
        }
    };
    assert_eq!(
        payload,
        &json!({"metadata": {"uid": "set-uid", "resourceVersion": "17"}, "spec": spec})
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
