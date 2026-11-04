use super::*;

fn stalled_scratch(
    ephemeral: bool,
    persisted_home: bool,
) -> (
    Workspace,
    Arc<DeleteMock>,
    KubernetesCoordinator,
    String,
    String,
    String,
) {
    let (_, builder, mock, coordinator) = setup();
    let (mut workspace, mut set, mut pod) = ready_pod_fixture();
    let names = builder.runtime_names(&workspace).unwrap().resources;
    let home_name = names.data_pvc_ordinal_zero();
    if persisted_home {
        bind_home(&mut workspace, &home_name);
        set["spec"]["template"]["spec"]["volumes"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "name":names.data_claim_template,"persistentVolumeClaim":{"claimName":home_name}
            }));
    }
    pod["metadata"]["resourceVersion"] = json!("pod-rv");
    pod["metadata"]["creationTimestamp"] = json!("2026-11-04T06:01:46Z");
    pod["spec"] = set["spec"]["template"]["spec"].clone();
    pod["spec"]["nodeName"] = json!("worker-1");
    if !persisted_home {
        pod["spec"]["volumes"].as_array_mut().unwrap().push(json!({
            "name":names.data_claim_template,"persistentVolumeClaim":{"claimName":home_name}
        }));
    }
    let scratch_name = if ephemeral {
        let scratch = pod["spec"]["volumes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|volume| volume["name"] == "workspace-scratch")
            .unwrap();
        *scratch = json!({"name":"workspace-scratch", "ephemeral":{
            "volumeClaimTemplate":{"spec":{"storageClassName":"local-scratch","accessModes":["ReadWriteOnce"]}}
        }});
        format!("{}-workspace-scratch", names.pod_ordinal_zero())
    } else {
        names.scratch_pvc()
    };
    let statuses = |containers: &Value, exit_code: i32| {
        containers.as_array().into_iter().flatten().map(|container| json!({
            "name":container["name"], "image":"example/workspace:1", "imageID":"example/image-id",
            "ready":false, "restartCount":0,
            "state":{"terminated":{"exitCode":exit_code,"reason":"Unknown"}}
        })).collect::<Vec<_>>()
    };
    pod["status"] = json!({"phase":"Running","conditions":[{"type":"Ready","status":"False"}],
        "containerStatuses":statuses(&pod["spec"]["containers"],255),
        "initContainerStatuses":statuses(&pod["spec"]["initContainers"],0)});
    let pod_path = format!("{}/{}", core("pods"), names.pod_ordinal_zero());
    let scratch_path = format!("{}/{}", core("persistentvolumeclaims"), scratch_name);
    let home_path = format!("{}/{}", core("persistentvolumeclaims"), home_name);
    let mut claim = json!({"apiVersion":"v1","kind":"PersistentVolumeClaim","metadata":{
        "name":scratch_name,"namespace":NAMESPACE,"uid":"empty-scratch-uid",
        "creationTimestamp":"2026-11-04T06:04:08Z","labels":ownership_labels("public-a",workspace.id)},
        "spec":{"storageClassName":"local-scratch"},"status":{"phase":"Pending"}});
    claim["metadata"]["labels"]["workspace.memeloop.dev/storage-role"] = json!("temporary");
    if ephemeral {
        claim["metadata"]["ownerReferences"] = json!([{"apiVersion":"v1","kind":"Pod",
            "name":names.pod_ordinal_zero(),"uid":pod["metadata"]["uid"],"controller":true}]);
    }
    let mut home = bound_claim();
    home["metadata"]["name"] = json!(home_name);
    home["metadata"]["creationTimestamp"] = json!("2026-11-04T06:00:00Z");
    home["metadata"]["labels"] = ownership_labels("public-a", workspace.id);
    mock.objects.lock().unwrap().extend([
        (format!("/apis/apps/v1/namespaces/{NAMESPACE}/statefulsets/{}",names.stateful_set),set),
        (pod_path.clone(),pod),
        (scratch_path.clone(),claim),
        (home_path.clone(),home),
        ("/api/v1/persistentvolumes/home-volume".into(),json!({"apiVersion":"v1","kind":"PersistentVolume",
            "metadata":{"name":"home-volume","uid":"original-home-pv-uid"},"spec":{"claimRef":{
                "name":home_name,"namespace":NAMESPACE,"uid":"home-uid"}},"status":{"phase":"Bound"}})),
        ("/apis/storage.k8s.io/v1/storageclasses/local-scratch".into(),json!({
            "apiVersion":"storage.k8s.io/v1","kind":"StorageClass","metadata":{"name":"local-scratch"},
            "provisioner":"topolvm.io","volumeBindingMode":"WaitForFirstConsumer"})),
    ]);
    (
        workspace,
        mock,
        coordinator,
        pod_path,
        scratch_path,
        home_path,
    )
}

#[tokio::test]
async fn recreated_pending_scratch_replaces_only_inactive_scheduled_pod_and_preserves_home() {
    for ephemeral in [true, false] {
        for persisted_home in [true, false] {
            let (workspace, mock, coordinator, pod, scratch, home) =
                stalled_scratch(ephemeral, persisted_home);
            let before = mock.objects.lock().unwrap().clone();
            assert!(
                !coordinator
                    .has_observed_replicas(&workspace, 1)
                    .await
                    .unwrap()
            );
            coordinator
                .recover_scratch_scheduling(&workspace)
                .await
                .unwrap();
            assert_eq!(mock.deletes(), [pod.clone()]);
            let mut expected = before.clone();
            expected.remove(&pod);
            assert_eq!(
                *mock.objects.lock().unwrap(),
                expected,
                "all claims, Home PV and history must remain"
            );
            assert!(mock.exists(&scratch));
            assert!(mock.exists(&home));
            let requests = mock.requests.lock().unwrap();
            let (_, _, options) = requests
                .iter()
                .find(|(method, _, _)| method == Method::DELETE)
                .unwrap();
            assert_eq!(
                options["preconditions"],
                json!({"uid":"current-pod-uid","resourceVersion":"pod-rv"})
            );
            assert!(
                options.get("gracePeriodSeconds").is_none(),
                "use normal termination"
            );
            assert!(
                requests
                    .iter()
                    .all(|(method, path, _)| method == Method::GET || path == &pod)
            );
        }
    }
}

#[tokio::test]
async fn scratch_recovery_refuses_active_or_unproven_volume_state() {
    for scenario in [
        "running",
        "waiting",
        "init-running",
        "unknown-status",
        "claim-owner",
        "selected-node",
        "bound",
        "immediate",
        "home-uid",
        "home-pv-claim-uid",
    ] {
        let (workspace, mock, coordinator, pod, scratch, home) = stalled_scratch(true, true);
        {
            let mut objects = mock.objects.lock().unwrap();
            match scenario {
                "running" => {
                    objects.get_mut(&pod).unwrap()["status"]["containerStatuses"][0]["state"] =
                        json!({"running":{}})
                }
                "waiting" => {
                    objects.get_mut(&pod).unwrap()["status"]["containerStatuses"][0]["state"] =
                        json!({"waiting":{"reason":"ContainerCreating"}})
                }
                "init-running" => {
                    objects.get_mut(&pod).unwrap()["status"]["initContainerStatuses"][0]["state"] =
                        json!({"running":{}})
                }
                "unknown-status" => {
                    objects.get_mut(&pod).unwrap()["status"]["containerStatuses"] = Value::Null
                }
                "claim-owner" => {
                    objects.get_mut(&scratch).unwrap()["metadata"]["ownerReferences"][0]["uid"] =
                        json!("another-pod")
                }
                "selected-node" => {
                    objects.get_mut(&scratch).unwrap()["metadata"]["annotations"] =
                        json!({"volume.kubernetes.io/selected-node":"worker-1"})
                }
                "bound" => {
                    objects.get_mut(&scratch).unwrap()["spec"]["volumeName"] =
                        json!("historical-scratch-volume")
                }
                "immediate" => {
                    objects
                        .get_mut("/apis/storage.k8s.io/v1/storageclasses/local-scratch")
                        .unwrap()["volumeBindingMode"] = json!("Immediate")
                }
                "home-uid" => {
                    objects.get_mut(&home).unwrap()["metadata"]["uid"] = json!("replacement-home")
                }
                "home-pv-claim-uid" => {
                    objects
                        .get_mut("/api/v1/persistentvolumes/home-volume")
                        .unwrap()["spec"]["claimRef"]["uid"] = json!("another-home")
                }
                _ => unreachable!(),
            }
        }
        let before = mock.objects.lock().unwrap().clone();
        let _ = coordinator.recover_scratch_scheduling(&workspace).await;
        assert!(mock.deletes().is_empty(), "{scenario}");
        assert_eq!(*mock.objects.lock().unwrap(), before, "{scenario}");
    }
}

#[tokio::test]
async fn pod_replacement_or_status_change_rejects_scratch_recovery_delete() {
    for replacement in [true, false] {
        let (workspace, mock, coordinator, pod, scratch, home) = stalled_scratch(true, true);
        if replacement {
            *mock.replace_before_delete.lock().unwrap() = Some(pod.clone());
        } else {
            *mock.update_before_delete.lock().unwrap() = Some(pod.clone());
        }
        coordinator
            .recover_scratch_scheduling(&workspace)
            .await
            .unwrap();
        assert!(mock.exists(&pod));
        assert!(mock.exists(&scratch));
        assert!(mock.exists(&home));
        let objects = mock.objects.lock().unwrap();
        assert_eq!(
            objects[&pod]["metadata"][if replacement {
                "uid"
            } else {
                "resourceVersion"
            }],
            json!(if replacement {
                "replacement-uid"
            } else {
                "new-version"
            })
        );
    }
}
