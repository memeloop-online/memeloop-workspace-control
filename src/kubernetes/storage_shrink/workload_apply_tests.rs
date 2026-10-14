use k8s_openapi::api::apps::v1::StatefulSet;
use serde_json::{Value, json};

use super::super::workload_apply::{project_owned_fields, retained_workload_apply};

fn apply_fields() -> Value {
    json!({
        "f:metadata": {
            "f:labels": {
                ".": {}, "f:app": {}, "f:workspace.memeloop.dev/installation": {}
            },
            "f:finalizers": {"v:\"ours-a\"": {}, "v:\"ours-z\"": {}}
        },
        "f:spec": {
            "f:replicas": {},
            "f:serviceName": {},
            "f:selector": {"f:matchLabels": {"f:app": {}}},
            "f:persistentVolumeClaimRetentionPolicy": {
                "f:whenDeleted": {}, "f:whenScaled": {}
            },
            "f:template": {
                "f:metadata": {
                    "f:labels": {"f:app": {}},
                    "f:annotations": {"f:ours.example/revision": {}}
                },
                "f:spec": {
                    "f:serviceAccountName": {},
                    "f:nodeSelector": {"f:pool": {}},
                    "f:affinity": {"f:nodeAffinity": {
                        "f:requiredDuringSchedulingIgnoredDuringExecution": {}
                    }},
                    "f:containers": {
                        "k:{\"name\":\"a-ttyd\"}": {
                            ".": {}, "f:image": {},
                            "f:env": {"k:{\"name\":\"KEEP\"}": {
                                ".": {}, "f:value": {}
                            }}
                        },
                        "k:{\"name\":\"z-workspace\"}": {
                            ".": {}, "f:image": {},
                            "f:env": {
                                "k:{\"name\":\"A_INJECTION\"}": {
                                    ".": {}, "f:valueFrom": {"f:secretKeyRef": {
                                        "f:name": {}, "f:key": {}
                                    }}
                                },
                                "k:{\"name\":\"Z_OVERRIDE\"}": {
                                    ".": {}, "f:value": {}
                                }
                            },
                            "f:ports": {
                                "k:{\"containerPort\":2222,\"protocol\":\"UDP\"}": {
                                    ".": {}, "f:name": {}
                                },
                                "k:{\"containerPort\":8080,\"protocol\":\"TCP\"}": {
                                    ".": {}, "f:name": {}
                                }
                            },
                            "f:volumeMounts": {
                                "k:{\"mountPath\":\"/home\"}": {
                                    ".": {}, "f:name": {}, "f:readOnly": {}
                                },
                                "k:{\"mountPath\":\"/scratch\"}": {
                                    ".": {}, "f:name": {}
                                }
                            },
                            "f:resources": {
                                "f:requests": {"f:cpu": {}, "f:memory": {}},
                                "f:limits": {"f:memory": {}}
                            }
                        }
                    },
                    "f:volumes": {
                        "k:{\"name\":\"a-home\"}": {
                            ".": {}, "f:persistentVolumeClaim": {
                                "f:claimName": {}, "f:readOnly": {}
                            }
                        },
                        "k:{\"name\":\"z-scratch\"}": {
                            ".": {}, "f:emptyDir": {"f:sizeLimit": {}}
                        }
                    }
                }
            },
            "f:volumeClaimTemplates": {}
        }
    })
}

fn fixture() -> Value {
    json!({
        "apiVersion": "apps/v1", "kind": "StatefulSet",
        "metadata": {
            "name": "workspace", "namespace": "workspaces",
            "uid": "set-uid", "resourceVersion": "17", "generation": 9,
            "labels": {
                "app": "workspace", "workspace.memeloop.dev/installation": "shrink-field-manager",
                "foreign.example/label": "do-not-copy"
            },
            "annotations": {"foreign.example/private": "private-controller-data"},
            "finalizers": ["ours-z", "foreign-finalizer", "ours-a"],
            "managedFields": [
                {
                    "manager": "memeloop-workspace-control", "operation": "Apply",
                    "apiVersion": "apps/v1", "fieldsType": "FieldsV1",
                    "fieldsV1": apply_fields()
                },
                {
                    "manager": "memeloop-workspace-control", "operation": "Update",
                    "apiVersion": "apps/v1", "fieldsType": "FieldsV1",
                    "fieldsV1": {"f:spec": {"f:revisionHistoryLimit": {}}}
                },
                {
                    "manager": "foreign-injector", "operation": "Apply",
                    "apiVersion": "apps/v1", "fieldsType": "FieldsV1", "fieldsV1": {
                        "f:metadata": {
                            "f:annotations": {"f:foreign.example/private": {}},
                            "f:labels": {"f:foreign.example/label": {}},
                            "f:finalizers": {"v:\"foreign-finalizer\"": {}}
                        },
                        "f:spec": {"f:template": {
                            "f:metadata": {"f:annotations": {"f:foreign.example/injected": {}}},
                            "f:spec": {
                                "f:priorityClassName": {},
                                "f:nodeSelector": {"f:foreign.example/region": {}},
                                "f:containers": {
                                    "k:{\"name\":\"foreign-sidecar\"}": {},
                                    "k:{\"name\":\"z-workspace\"}": {
                                        "f:imagePullPolicy": {},
                                        "f:env": {"k:{\"name\":\"FOREIGN_TOKEN\"}": {
                                            ".": {}, "f:name": {}, "f:value": {}
                                        }}
                                    }
                                },
                                "f:volumes": {"k:{\"name\":\"foreign-secret\"}": {}}
                            }
                        }}
                    }
                },
                {
                    "manager": "kube-controller-manager", "operation": "Update",
                    "apiVersion": "apps/v1", "subresource": "status", "fieldsType": "FieldsV1",
                    "fieldsV1": {"f:status": {"f:observedGeneration": {}, "f:replicas": {}}}
                }
            ]
        },
        "spec": {
            "replicas": 3, "serviceName": "workspace-headless",
            "revisionHistoryLimit": 21,
            "selector": {"matchLabels": {"app": "workspace"}},
            "persistentVolumeClaimRetentionPolicy": {
                "whenDeleted": "Delete", "whenScaled": "Delete"
            },
            "template": {
                "metadata": {
                    "labels": {"app": "workspace", "foreign.example/label": "not-owned"},
                    "annotations": {
                        "ours.example/revision": "keep", "foreign.example/injected": "not-owned"
                    }
                },
                "spec": {
                    "serviceAccountName": "workspace",
                    "priorityClassName": "foreign-priority",
                    "nodeSelector": {"pool": "dedicated", "foreign.example/region": "not-owned"},
                    "affinity": {"nodeAffinity": {
                        "requiredDuringSchedulingIgnoredDuringExecution": {
                            "nodeSelectorTerms": [{"matchExpressions": [{
                                "key": "pool", "operator": "In", "values": ["dedicated", "fallback"]
                            }]}]
                        }
                    }},
                    "containers": [
                        {
                            "name": "z-workspace", "image": "workspace:current-override",
                            "imagePullPolicy": "IfNotPresent",
                            "env": [
                                {"name": "Z_OVERRIDE", "value": "ours-override"},
                                {"name": "FOREIGN_TOKEN", "value": "private-foreign-value"},
                                {"name": "A_INJECTION", "valueFrom": {"secretKeyRef": {
                                    "name": "workspace-injection", "key": "token"
                                }}}
                            ],
                            "ports": [
                                {"name": "web", "containerPort": 8080, "protocol": "TCP"},
                                {"name": "shell", "containerPort": 2222, "protocol": "UDP"}
                            ],
                            "volumeMounts": [
                                {"name": "z-scratch", "mountPath": "/scratch"},
                                {"name": "a-home", "mountPath": "/home", "readOnly": false}
                            ],
                            "resources": {
                                "requests": {"cpu": "500m", "memory": "1Gi"},
                                "limits": {"memory": "2Gi"}
                            }
                        },
                        {"name": "foreign-sidecar", "image": "foreign:latest"},
                        {
                            "name": "a-ttyd", "image": "ttyd:current-injected",
                            "env": [{"name": "KEEP", "value": "ours-ttyd"}]
                        }
                    ],
                    "volumes": [
                        {"name": "z-scratch", "emptyDir": {"sizeLimit": "8Gi"}},
                        {"name": "foreign-secret", "secret": {"secretName": "foreign"}},
                        {"name": "a-home", "persistentVolumeClaim": {
                            "claimName": "existing-home", "readOnly": false
                        }}
                    ]
                }
            },
            "volumeClaimTemplates": [
                {
                    "metadata": {"name": "z-scratch", "labels": {"storage-role": "scratch"}},
                    "spec": {
                        "accessModes": ["ReadWriteOnce"], "storageClassName": "longhorn",
                        "volumeMode": "Filesystem", "resources": {"requests": {"storage": "8Gi"}}
                    }
                },
                {
                    "metadata": {"name": "a-home", "labels": {"storage-role": "home"}},
                    "spec": {
                        "accessModes": ["ReadWriteOnce"], "storageClassName": "longhorn",
                        "volumeMode": "Filesystem", "resources": {"requests": {"storage": "60Gi"}}
                    }
                }
            ]
        },
        "status": {"observedGeneration": 9, "replicas": 3}
    })
}

fn retained(value: Value, stop: bool) -> Result<Value, String> {
    let _: StatefulSet = serde_json::from_value(value.clone())
        .expect("fixture must deserialize as a real StatefulSet before testing ownership");
    retained_workload_apply(&value, stop)
}

fn expected_apply(stop: bool) -> Value {
    let source = fixture();
    let mut spec = source["spec"].clone();
    spec.as_object_mut().unwrap().remove("revisionHistoryLimit");
    spec["persistentVolumeClaimRetentionPolicy"] = json!({
        "whenDeleted": "Retain", "whenScaled": "Retain"
    });
    if stop {
        spec["replicas"] = json!(0);
    }
    let template = &mut spec["template"];
    template["metadata"]["labels"]
        .as_object_mut()
        .unwrap()
        .remove("foreign.example/label");
    template["metadata"]["annotations"]
        .as_object_mut()
        .unwrap()
        .remove("foreign.example/injected");
    let pod = &mut template["spec"];
    pod.as_object_mut().unwrap().remove("priorityClassName");
    pod["nodeSelector"]
        .as_object_mut()
        .unwrap()
        .remove("foreign.example/region");
    pod["containers"].as_array_mut().unwrap().remove(1);
    let container = &mut pod["containers"][0];
    container.as_object_mut().unwrap().remove("imagePullPolicy");
    container["env"].as_array_mut().unwrap().remove(1);
    pod["volumes"].as_array_mut().unwrap().remove(1);
    json!({
        "apiVersion": "apps/v1", "kind": "StatefulSet",
        "metadata": {
            "name": "workspace", "namespace": "workspaces", "uid": "set-uid",
            "resourceVersion": "17", "finalizers": ["ours-z", "ours-a"],
            "labels": {
                "app": "workspace", "workspace.memeloop.dev/installation": "shrink-field-manager"
            }
        },
        "spec": spec
    })
}

#[test]
fn retaining_reapplies_all_current_owned_fields_without_foreign_fields() {
    let source = fixture();
    let apply = retained(source.clone(), false).unwrap();
    assert_eq!(apply, expected_apply(false));
    assert_eq!(apply["spec"]["replicas"], source["spec"]["replicas"]);
    for field in ["selector", "serviceName", "volumeClaimTemplates"] {
        assert_eq!(
            apply["spec"][field], source["spec"][field],
            "must preserve {field}"
        );
    }
    assert!(apply["metadata"].get("annotations").is_none());
    assert!(apply["metadata"].get("managedFields").is_none());
    assert!(apply["metadata"].get("generation").is_none());
    assert!(apply.get("status").is_none());
    assert!(!apply.to_string().contains("private-foreign-value"));
}

#[test]
fn stopping_changes_only_owned_replicas_and_retention_policy() {
    let retain = retained(fixture(), false).unwrap();
    let stop = retained(fixture(), true).unwrap();
    assert_eq!(stop, expected_apply(true));
    let mut expected = retain;
    expected["spec"]["replicas"] = json!(0);
    assert_eq!(stop, expected);
    assert_eq!(
        stop["spec"]["persistentVolumeClaimRetentionPolicy"],
        json!({
            "whenDeleted": "Retain", "whenScaled": "Retain"
        })
    );
}

#[test]
fn keyed_and_set_lists_preserve_source_order_and_merge_keys() {
    let apply = retained(fixture(), false).unwrap();
    let pod = &apply["spec"]["template"]["spec"];
    assert_eq!(apply["metadata"]["finalizers"], json!(["ours-z", "ours-a"]));
    assert_eq!(
        pod["containers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["z-workspace", "a-ttyd"]
    );
    assert_eq!(
        pod["volumes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["z-scratch", "a-home"]
    );
    let container = &pod["containers"][0];
    assert_eq!(
        container["env"][0],
        json!({"name": "Z_OVERRIDE", "value": "ours-override"})
    );
    assert_eq!(container["env"][1]["name"], "A_INJECTION");
    assert_eq!(
        container["ports"],
        json!([
            {"name": "web", "containerPort": 8080, "protocol": "TCP"},
            {"name": "shell", "containerPort": 2222, "protocol": "UDP"}
        ])
    );
    assert_eq!(container["volumeMounts"][0]["mountPath"], "/scratch");
    assert_eq!(container["volumeMounts"][1]["mountPath"], "/home");
}

#[test]
fn missing_fields_omitted_by_the_api_are_skipped() {
    let mut source = fixture();
    let pod = &mut source["spec"]["template"]["spec"];
    pod["containers"][2].as_object_mut().unwrap().remove("env");
    pod["containers"][0]["volumeMounts"][1]
        .as_object_mut()
        .unwrap()
        .remove("readOnly");
    pod["volumes"][2]["persistentVolumeClaim"]
        .as_object_mut()
        .unwrap()
        .remove("readOnly");
    let mut expected = expected_apply(false);
    let pod = &mut expected["spec"]["template"]["spec"];
    pod["containers"][1].as_object_mut().unwrap().remove("env");
    pod["containers"][0]["volumeMounts"][1]
        .as_object_mut()
        .unwrap()
        .remove("readOnly");
    pod["volumes"][1]["persistentVolumeClaim"]
        .as_object_mut()
        .unwrap()
        .remove("readOnly");
    assert_eq!(retained(source, false).unwrap(), expected);
}

#[test]
fn same_name_update_entry_is_not_used_instead_of_apply_ownership() {
    let mut source = fixture();
    source["metadata"]["managedFields"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    assert!(retained(source, false).is_err());
}

#[test]
fn missing_or_duplicate_apply_ownership_fails_closed() {
    for missing in [Value::Null, json!([])] {
        let mut source = fixture();
        if missing.is_null() {
            source["metadata"]
                .as_object_mut()
                .unwrap()
                .remove("managedFields");
        } else {
            source["metadata"]["managedFields"] = missing;
        }
        assert!(retained(source, false).is_err());
    }
    let mut source = fixture();
    let duplicate = source["metadata"]["managedFields"][0].clone();
    source["metadata"]["managedFields"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    assert!(retained(source, false).is_err());
}

#[test]
fn apply_entry_must_match_manager_operation_version_type_and_subresource() {
    for (field, value) in [
        ("manager", "another-controller"),
        ("operation", "Update"),
        ("apiVersion", "apps/v1beta1"),
        ("fieldsType", "FieldsV2"),
        ("subresource", "status"),
    ] {
        let mut source = fixture();
        source["metadata"]["managedFields"][0][field] = json!(value);
        assert!(
            retained(source, false).is_err(),
            "invalid {field} must fail closed"
        );
    }
}

#[test]
fn unrelated_apply_entries_are_ignored_and_empty_subresource_is_accepted() {
    let mut source = fixture();
    source["metadata"]["managedFields"][0]["subresource"] = json!("");
    let status_entry = json!({
        "manager": "memeloop-workspace-control", "operation": "Apply", "apiVersion": "apps/v1",
        "subresource": "status", "fieldsType": "FieldsV1", "fieldsV1": {"f:status": {}}
    });
    let mut old_version = source["metadata"]["managedFields"][0].clone();
    old_version["apiVersion"] = json!("apps/v1beta1");
    source["metadata"]["managedFields"]
        .as_array_mut()
        .unwrap()
        .extend([status_entry, old_version]);
    assert_eq!(retained(source, false).unwrap(), expected_apply(false));
}

#[test]
fn all_compare_and_swap_identity_fields_are_required_and_nonempty() {
    for field in ["name", "namespace", "uid", "resourceVersion"] {
        for empty in [false, true] {
            let mut source = fixture();
            if empty {
                source["metadata"][field] = json!("");
            } else {
                source["metadata"].as_object_mut().unwrap().remove(field);
            }
            assert!(
                retained(source, false).is_err(),
                "missing or empty {field} must fail closed"
            );
        }
    }
}

#[test]
fn missing_or_non_object_fields_v1_fails_closed() {
    let mut source = fixture();
    source["metadata"]["managedFields"][0]
        .as_object_mut()
        .unwrap()
        .remove("fieldsV1");
    assert!(retained(source, false).is_err());
    for fields in [Value::Null, json!([]), json!("invalid"), json!({})] {
        let mut source = fixture();
        source["metadata"]["managedFields"][0]["fieldsV1"] = fields;
        assert!(retained(source, false).is_err());
    }
}

#[test]
fn malformed_keyed_list_selectors_fail_closed_on_real_containers() {
    for selector in ["k:{", "k:[]", "k:{}", "k:null", "k:\"z-workspace\"", "i:0"] {
        let mut source = fixture();
        source["metadata"]["managedFields"][0]["fieldsV1"]["f:spec"]["f:template"]["f:spec"]["f:containers"] =
            json!({(selector): {".": {}, "f:image": {}}});
        assert!(
            retained(source, false).is_err(),
            "selector {selector} must fail closed"
        );
    }
}

#[test]
fn malformed_set_list_selectors_and_nonempty_children_fail_closed() {
    for fields in [
        json!({"v:not-json": {}}),
        json!({"v:\"ours-z\"": {"f:name": {}}}),
        json!({"v:\"ours-z\"": []}),
        json!({"i:0": {}}),
    ] {
        let mut source = fixture();
        source["metadata"]["managedFields"][0]["fieldsV1"]["f:metadata"]["f:finalizers"] = fields;
        assert!(retained(source, false).is_err());
    }
}

#[test]
fn duplicate_live_keyed_or_set_list_items_fail_closed() {
    let mut source = fixture();
    let env = &mut source["spec"]["template"]["spec"]["containers"][0]["env"];
    let duplicate = env[0].clone();
    env.as_array_mut().unwrap().push(duplicate);
    assert!(retained(source, false).is_err());
    let mut source = fixture();
    source["metadata"]["finalizers"]
        .as_array_mut()
        .unwrap()
        .push(json!("ours-z"));
    assert!(retained(source, false).is_err());
}

#[test]
fn ambiguous_selectors_for_one_live_list_item_fail_closed() {
    let mut source = fixture();
    source["metadata"]["managedFields"][0]["fieldsV1"]["f:spec"]["f:template"]["f:spec"]["f:containers"] = json!({
        "k:{\"name\":\"z-workspace\"}": {"f:image": {}},
        "k:{ \"name\": \"z-workspace\" }": {"f:image": {}}
    });
    assert!(retained(source, false).is_err());
}

#[test]
fn missing_owned_list_items_fail_closed_instead_of_dropping_owned_configuration() {
    let mut source = fixture();
    source["spec"]["template"]["spec"]["containers"][0]["env"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    assert!(retained(source, false).is_err());
}

#[test]
fn unknown_fieldset_keys_and_invalid_dot_markers_fail_closed() {
    for fields in [
        json!({"unknown:replicas": {}}),
        json!({"i:0": {}}),
        json!({".": {"f:replicas": {}}}),
        json!({".": []}),
        json!({"f:replicas": []}),
    ] {
        let mut source = fixture();
        source["metadata"]["managedFields"][0]["fieldsV1"]["f:spec"] = fields;
        assert!(retained(source, false).is_err());
    }
}

#[test]
fn ownership_of_status_or_other_root_fields_is_rejected() {
    for field in ["status", "kind", "apiVersion"] {
        let mut source = fixture();
        source["metadata"]["managedFields"][0]["fieldsV1"][format!("f:{field}")] = json!({});
        assert!(retained(source, false).is_err(), "must not reapply {field}");
    }
}

#[test]
fn projection_depth_limit_accepts_64_and_rejects_65_without_invalid_typed_fixtures() {
    for depth in [64, 65] {
        let mut current = json!("leaf");
        let mut fields = json!({});
        for _ in 0..depth {
            current = json!({"child": current});
            fields = json!({"f:child": fields});
        }
        let projected = project_owned_fields(&current, &fields, 0);
        if depth == 64 {
            assert_eq!(projected.unwrap(), current);
        } else {
            assert!(projected.is_err());
        }
    }
    assert!(project_owned_fields(&json!({}), &json!({}), 65).is_err());
}

#[test]
fn nonempty_granular_metadata_labels_cannot_be_owned_as_an_atomic_leaf() {
    let mut source = fixture();
    assert_eq!(
        source["metadata"]["labels"]["foreign.example/label"],
        "do-not-copy"
    );
    source["metadata"]["managedFields"][0]["fieldsV1"]["f:metadata"]["f:labels"] = json!({});
    assert!(retained(source, false).is_err());
}

#[test]
fn nonempty_container_cannot_be_copied_from_an_empty_keyed_fieldset() {
    let mut source = fixture();
    source["metadata"]["managedFields"][0]["fieldsV1"]["f:spec"]["f:template"]["f:spec"]["f:containers"] = json!({
        "k:{\"name\":\"z-workspace\"}": {}
    });
    assert!(retained(source, false).is_err());
}

#[test]
fn nonempty_granular_container_list_cannot_be_owned_as_an_atomic_leaf() {
    let mut source = fixture();
    source["metadata"]["managedFields"][0]["fieldsV1"]["f:spec"]["f:template"]["f:spec"]["f:containers"] =
        json!({});
    assert!(retained(source, false).is_err());
}

#[test]
fn catalog_atomic_selector_claim_templates_and_commands_preserve_only_owned_fields() {
    let mut source = fixture();
    source["metadata"]["managedFields"][0]["fieldsV1"]["f:spec"]["f:selector"] = json!({});
    let container = &mut source["spec"]["template"]["spec"]["containers"][0];
    container["command"] = json!(["/bin/sh", "-c"]);
    container["args"] = json!(["run-current-injection"]);
    let owned = &mut source["metadata"]["managedFields"][0]["fieldsV1"]["f:spec"]["f:template"]["f:spec"]
        ["f:containers"]["k:{\"name\":\"z-workspace\"}"];
    owned["f:command"] = json!({});
    owned["f:args"] = json!({});
    let apply = retained(source.clone(), false).unwrap();
    let mut expected = expected_apply(false);
    expected["spec"]["template"]["spec"]["containers"][0]["command"] =
        source["spec"]["template"]["spec"]["containers"][0]["command"].clone();
    expected["spec"]["template"]["spec"]["containers"][0]["args"] =
        source["spec"]["template"]["spec"]["containers"][0]["args"].clone();
    assert_eq!(apply, expected);
    assert_eq!(apply["spec"]["selector"], source["spec"]["selector"]);
    assert_eq!(
        apply["spec"]["volumeClaimTemplates"],
        source["spec"]["volumeClaimTemplates"]
    );
    assert!(apply["metadata"].get("annotations").is_none());
    assert!(apply["metadata"].get("managedFields").is_none());
    assert!(apply.get("status").is_none());
    assert!(!apply.to_string().contains("private-foreign-value"));
}

#[test]
fn raw_owned_future_scalar_fields_survive_even_when_typed_statefulset_drops_them() {
    for value in [
        json!("future-owned-value"),
        json!(42),
        json!(true),
        Value::Null,
    ] {
        let mut source = fixture();
        source["spec"]["template"]["spec"]["futureSafeScalar"] = value.clone();
        source["spec"]["template"]["spec"]["futureForeignScalar"] = json!("do-not-copy-future");
        source["metadata"]["managedFields"][0]["fieldsV1"]["f:spec"]["f:template"]["f:spec"]["f:futureSafeScalar"] =
            json!({});
        source["metadata"]["managedFields"][2]["fieldsV1"]["f:spec"]["f:template"]["f:spec"]["f:futureForeignScalar"] =
            json!({});
        let typed: StatefulSet = serde_json::from_value(source.clone()).unwrap();
        let roundtrip = serde_json::to_value(typed).unwrap();
        assert!(
            roundtrip["spec"]["template"]["spec"]
                .get("futureSafeScalar")
                .is_none()
        );
        let apply = retained(source, false).unwrap();
        let mut expected = expected_apply(false);
        expected["spec"]["template"]["spec"]["futureSafeScalar"] = value.clone();
        assert_eq!(apply, expected);
        assert_eq!(
            apply["spec"]["template"]["spec"].get("futureSafeScalar"),
            Some(&value)
        );
        assert!(
            apply["spec"]["template"]["spec"]
                .get("futureForeignScalar")
                .is_none()
        );
    }
}

#[test]
fn unknown_nonempty_object_and_array_leaves_fail_closed_on_raw_fields() {
    for value in [
        json!({"foreignNestedValue": "do-not-copy"}),
        json!(["do-not-copy"]),
    ] {
        let mut source = fixture();
        source["spec"]["template"]["spec"]["futureUnsafeCollection"] = value;
        source["metadata"]["managedFields"][0]["fieldsV1"]["f:spec"]["f:template"]["f:spec"]["f:futureUnsafeCollection"] =
            json!({});
        assert!(retained(source, false).is_err());
    }
}

#[test]
fn unknown_empty_object_and_array_leaves_are_safe_to_preserve() {
    for value in [json!({}), json!([])] {
        let mut source = fixture();
        source["spec"]["template"]["spec"]["futureEmptyCollection"] = value.clone();
        source["metadata"]["managedFields"][0]["fieldsV1"]["f:spec"]["f:template"]["f:spec"]["f:futureEmptyCollection"] =
            json!({});
        let mut expected = expected_apply(false);
        expected["spec"]["template"]["spec"]["futureEmptyCollection"] = value;
        assert_eq!(retained(source, false).unwrap(), expected);
    }
}

#[test]
fn other_replica_owners_are_rejected_even_when_stopped_or_coowned() {
    for (manager, operation, subresource) in [
        ("memeloop-workspace-control", "Update", ""),
        ("memeloop-workspace-control", "Update", "scale"),
        ("external-scaler", "Apply", ""),
    ] {
        for replicas in [0, 3] {
            for stop in [false, true] {
                let mut source = fixture();
                source["spec"]["replicas"] = json!(replicas);
                source["metadata"]["managedFields"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({
                        "manager": manager, "operation": operation,
                        "apiVersion": "apps/v1", "subresource": subresource,
                        "fieldsType": "FieldsV1",
                        "fieldsV1": {"f:spec": {"f:replicas": {}}}
                    }));
                assert!(retained(source, stop).is_err());
            }
        }
    }
}

#[test]
fn retention_changes_preserve_other_owned_raw_policy_fields() {
    let mut source = fixture();
    source["spec"]["persistentVolumeClaimRetentionPolicy"]["futureOwnedHint"] = json!("keep");
    source["metadata"]["managedFields"][0]["fieldsV1"]["f:spec"]["f:persistentVolumeClaimRetentionPolicy"] = json!({
        "f:whenDeleted": {}, "f:whenScaled": {}, "f:futureOwnedHint": {}
    });
    let apply = retained(source, false).unwrap();
    assert_eq!(
        apply["spec"]["persistentVolumeClaimRetentionPolicy"],
        json!({"whenDeleted": "Retain", "whenScaled": "Retain", "futureOwnedHint": "keep"})
    );
}

#[test]
fn managed_fields_cannot_be_reapplied_even_if_the_fieldset_claims_ownership() {
    let mut source = fixture();
    source["metadata"]["managedFields"][0]["fieldsV1"]["f:metadata"]["f:managedFields"] = json!({});
    assert!(retained(source, false).is_err());
}
