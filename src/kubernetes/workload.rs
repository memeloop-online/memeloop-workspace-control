use std::collections::BTreeMap;

use k8s_openapi::{
    api::{
        apps::v1::{StatefulSet, StatefulSetSpec},
        core::v1::{
            ConfigMapVolumeSource, Container, ContainerPort, EmptyDirVolumeSource,
            PersistentVolumeClaim, PersistentVolumeClaimSpec, PodSpec, PodTemplateSpec,
            ResourceRequirements, SecretVolumeSource, Volume, VolumeResourceRequirements,
        },
    },
    apimachinery::pkg::{
        api::resource::Quantity,
        apis::meta::v1::{LabelSelector, ObjectMeta},
    },
};

use crate::{
    templates::WorkspaceStoragePolicy,
    workspace_runtime::{WorkspaceResourceNames, WorkspaceRuntimeNames},
    workspaces::Workspace,
};

use super::{
    ResourceBuilder, namespaced_metadata,
    resource_helpers::{mount, pod_labels},
    workspace_pod::WorkspacePod,
};

pub(super) fn stateful_set(
    builder: &ResourceBuilder,
    runtime: &WorkspaceRuntimeNames,
    labels: &BTreeMap<String, String>,
    template_labels: &BTreeMap<String, String>,
    workspace: &Workspace,
    replicas: i32,
) -> StatefulSet {
    let names = &runtime.resources;
    let stable_labels = builder.labels(workspace.id);
    let pod = WorkspacePod::from_template(&workspace.template);
    let containers = containers(builder, pod, workspace, names, &runtime.route_key);
    StatefulSet {
        metadata: namespaced_metadata(&names.stateful_set, &runtime.namespace, labels),
        spec: Some(StatefulSetSpec {
            replicas: Some(replicas),
            service_name: Some(names.service.clone()),
            selector: LabelSelector {
                match_labels: Some(pod_labels(&stable_labels)),
                ..LabelSelector::default()
            },
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(template_labels.clone()),
                    annotations: Some(BTreeMap::from([(
                        "workspace.memeloop.dev/generation".to_owned(),
                        workspace.generation.to_string(),
                    )])),
                    ..ObjectMeta::default()
                }),
                spec: Some(pod_spec(pod, workspace, containers, &names)),
            },
            volume_claim_templates: Some(vec![workspace_claim(
                builder,
                stable_labels,
                workspace,
                &names,
            )]),
            ..StatefulSetSpec::default()
        }),
        ..StatefulSet::default()
    }
}

fn containers(
    builder: &ResourceBuilder,
    pod: WorkspacePod<'_>,
    workspace: &Workspace,
    names: &WorkspaceResourceNames,
    route_key: &str,
) -> Vec<Container> {
    let mut containers = vec![pod.workspace_container(
        &workspace.template.image,
        workspace_resources(pod, workspace),
        names,
    )];
    if let Some(buildkit) = pod.buildkit_container() {
        containers.push(buildkit);
    }
    containers.push(ttyd_container(builder, pod, route_key));
    containers
}

fn pod_spec(
    pod: WorkspacePod<'_>,
    workspace: &Workspace,
    containers: Vec<Container>,
    names: &WorkspaceResourceNames,
) -> PodSpec {
    let mut init_containers = vec![pod.workspace_init_container(&workspace.template.image, names)];
    if let Some(buildkit_bootstrap) = pod.buildkit_bootstrap_container() {
        init_containers.push(buildkit_bootstrap);
    }
    let cluster_access = workspace.template.cluster_access;
    PodSpec {
        automount_service_account_token: Some(cluster_access),
        service_account_name: cluster_access.then(|| names.service_account.clone()),
        init_containers: Some(init_containers),
        containers,
        affinity: pod.affinity(),
        node_selector: pod.node_selector(),
        security_context: pod.pod_security_context(),
        image_pull_secrets: pod.image_pull_secrets(),
        volumes: Some(workspace_volumes(&workspace.template.storage_policy, names)),
        ..PodSpec::default()
    }
}

fn workspace_resources(pod: WorkspacePod<'_>, workspace: &Workspace) -> ResourceRequirements {
    let mut requests = pod.resource_requests();
    let mut limits = pod.resource_limits();
    if workspace.template.resources.gpu_count > 0 {
        let quantity = Quantity(workspace.template.resources.gpu_count.to_string());
        requests.insert("nvidia.com/gpu".to_owned(), quantity.clone());
        limits.insert("nvidia.com/gpu".to_owned(), quantity);
    }
    ResourceRequirements {
        requests: Some(requests),
        limits: Some(limits),
        ..ResourceRequirements::default()
    }
}

fn ttyd_container(builder: &ResourceBuilder, pod: WorkspacePod<'_>, route_key: &str) -> Container {
    Container {
        name: "ttyd".to_owned(),
        image: Some(builder.ttyd_image.clone()),
        command: Some(vec!["/usr/bin/ttyd".to_owned()]),
        args: Some(vec![
            "--port".to_owned(),
            "7681".to_owned(),
            "--writable".to_owned(),
            "--base-path".to_owned(),
            format!("/shell/{route_key}"),
            "/usr/bin/ssh".to_owned(),
            "-p".to_owned(),
            "2222".to_owned(),
            "-o".to_owned(),
            "StrictHostKeyChecking=yes".to_owned(),
            "-o".to_owned(),
            "UserKnownHostsFile=/etc/ssh/platform/known_hosts".to_owned(),
            "-o".to_owned(),
            "BatchMode=yes".to_owned(),
            "-i".to_owned(),
            "/etc/ssh/platform/ttyd_client_key".to_owned(),
            format!("{}@127.0.0.1", pod.login_user),
        ]),
        ports: Some(vec![ContainerPort {
            container_port: 7681,
            name: Some("web-shell".to_owned()),
            protocol: Some("TCP".to_owned()),
            ..ContainerPort::default()
        }]),
        volume_mounts: Some(vec![
            mount("runtime-ssh", "/etc/ssh/platform", true),
            mount("ttyd-tmp", "/tmp", false),
            mount("ttyd-tmp", "/var/tmp", false),
        ]),
        resources: Some(ResourceRequirements {
            requests: Some(BTreeMap::from([
                ("cpu".to_owned(), Quantity("10m".to_owned())),
                ("memory".to_owned(), Quantity("16Mi".to_owned())),
            ])),
            limits: Some(BTreeMap::from([
                ("cpu".to_owned(), Quantity("100m".to_owned())),
                ("memory".to_owned(), Quantity("128Mi".to_owned())),
            ])),
            ..ResourceRequirements::default()
        }),
        ..Container::default()
    }
}

fn workspace_volumes(
    policy: &WorkspaceStoragePolicy,
    names: &WorkspaceResourceNames,
) -> Vec<Volume> {
    vec![
        Volume {
            name: "ssh-identity".to_owned(),
            secret: Some(SecretVolumeSource {
                secret_name: Some(names.ssh_identity_secret.clone()),
                default_mode: Some(0o400),
                ..SecretVolumeSource::default()
            }),
            ..Volume::default()
        },
        Volume {
            name: "workspace-files-secret".to_owned(),
            secret: Some(SecretVolumeSource {
                secret_name: Some(names.files_secret.clone()),
                ..SecretVolumeSource::default()
            }),
            ..Volume::default()
        },
        Volume {
            name: "workspace-files-config".to_owned(),
            config_map: Some(ConfigMapVolumeSource {
                name: names.files_config_map.clone(),
                ..ConfigMapVolumeSource::default()
            }),
            ..Volume::default()
        },
        Volume {
            name: "workspace-config".to_owned(),
            config_map: Some(ConfigMapVolumeSource {
                name: names.workspace_config.clone(),
                default_mode: Some(0o555),
                ..ConfigMapVolumeSource::default()
            }),
            ..Volume::default()
        },
        Volume {
            name: "runtime-ssh".to_owned(),
            empty_dir: Some(bounded_empty_dir("128Mi", Some("Memory"))),
            ..Volume::default()
        },
        Volume {
            name: "runtime-tmp".to_owned(),
            empty_dir: Some(bounded_empty_dir(
                &format!("{}Mi", policy.runtime_tmp_memory_mib),
                Some("Memory"),
            )),
            ..Volume::default()
        },
        Volume {
            name: "ttyd-tmp".to_owned(),
            empty_dir: Some(bounded_empty_dir("128Mi", Some("Memory"))),
            ..Volume::default()
        },
        Volume {
            name: "build-scratch".to_owned(),
            empty_dir: Some(bounded_empty_dir(
                &format!("{}Gi", policy.build_scratch_gib),
                None,
            )),
            ..Volume::default()
        },
        Volume {
            name: "buildkit-cache".to_owned(),
            empty_dir: Some(bounded_empty_dir(
                &format!("{}Gi", policy.buildkit_cache_gib),
                None,
            )),
            ..Volume::default()
        },
        Volume {
            name: "codex-scratch".to_owned(),
            empty_dir: Some(bounded_empty_dir(
                &format!("{}Gi", policy.codex_scratch_gib),
                None,
            )),
            ..Volume::default()
        },
    ]
}

fn bounded_empty_dir(size: &str, medium: Option<&str>) -> EmptyDirVolumeSource {
    EmptyDirVolumeSource {
        medium: medium.map(str::to_owned),
        size_limit: Some(Quantity(size.to_owned())),
    }
}

fn workspace_claim(
    builder: &ResourceBuilder,
    stable_labels: BTreeMap<String, String>,
    workspace: &Workspace,
    names: &WorkspaceResourceNames,
) -> PersistentVolumeClaim {
    PersistentVolumeClaim {
        metadata: ObjectMeta {
            name: Some(names.data_claim_template.clone()),
            labels: Some(stable_labels),
            ..ObjectMeta::default()
        },
        spec: Some(PersistentVolumeClaimSpec {
            access_modes: Some(vec!["ReadWriteOnce".to_owned()]),
            storage_class_name: builder.storage_class_name.clone(),
            resources: Some(VolumeResourceRequirements {
                requests: Some(BTreeMap::from([(
                    "storage".to_owned(),
                    Quantity(format!("{}Gi", workspace.template.resources.disk_gib)),
                )])),
                ..VolumeResourceRequirements::default()
            }),
            ..PersistentVolumeClaimSpec::default()
        }),
        ..PersistentVolumeClaim::default()
    }
}
