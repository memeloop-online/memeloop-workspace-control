//! Export the real product renderer for the disposable CSI workflow.
use std::{collections::BTreeMap, path::Path};

use k8s_openapi::api::core::v1::Pod;
use memeloop_workspace_control::{
    kubernetes::ResourceBuilder,
    quota::Resources,
    templates::WorkspaceTemplateSpec,
    workspace_runtime::{WorkspaceRuntimeIdentity, WorkspaceRuntimeNames},
    workspaces::{AccessMode, Workspace, WorkspaceState},
};
use serde_json::json;
use uuid::Uuid;

#[test]
fn export_actual_rust_storage_cases() {
    let Some(directory) = std::env::var_os("MWC_PRODUCT_STORAGE_FIXTURES") else {
        return;
    };
    let directory = Path::new(&directory);
    std::fs::create_dir_all(directory).unwrap();
    let builder = ResourceBuilder {
        installation_id: "storage-ci".parse().unwrap(),
        ttyd_image: "ghcr.io/memeloop-online/memeloop-workspace-control-ttyd@sha256:cc408d2cb9c3fe457b206cffae4a15264ae15f037eac0b1f4dde86058d499012".into(),
        buildkit_image: "moby/buildkit:v0.33.0-rootless@sha256:80b15f0735e87bab7bf59ec4d695dfb4a7cfb25521cf56dc75d6f256285b63ef".into(),
        ttyd_mtls: None,
        higress_namespace: "higress-system".into(),
        higress_pod_labels: BTreeMap::new(),
        higress_source_cidrs: Vec::new(),
        internet_egress: None,
        jump_host_namespace: "workspace-access".into(),
        jump_host_pod_labels: BTreeMap::new(),
        storage_class_name: Some("mwc-ci-home".into()),
        scratch_storage_class_name: "mwc-ci-scratch".into(),
        web_shell_domain: None,
        port_mapping_domain: None,
        higress_gateway_name: "higress-gateway".into(),
        higress_https_section_name: "https".into(),
        internal_ssh_node_port_enabled: false,
    };
    for buildkit in [false, true] {
        let id = Uuid::parse_str(if buildkit {
            "00000000-0000-7000-8000-00000000c502"
        } else {
            "00000000-0000-7000-8000-00000000c501"
        })
        .unwrap();
        let short_id = format!("{:016x}", id.as_u128() as u64);
        let mut template = WorkspaceTemplateSpec::standard(
            "harbor.k3s.onetwo.website/library/rust-dev:20261027-74d5ef8@sha256:d50798f0f8345d7295c303ec0f04b1f1e064b7d6ac9b074d87065a488957360f",
            AccessMode::Internal,
            Resources {
                cpu_millis: 2000,
                memory_mib: 2048,
                gpu_count: 0,
                disk_gib: 20,
            },
        );
        template.workspace_user = "user".into();
        template.workspace_home = "/home/user".into();
        template.buildkit = buildkit;
        template.pod_requests.cpu_millis = 200;
        template.pod_requests.memory_mib = 512;
        template.storage_policy.temporary_storage_gib = 10;
        template.runtime_class_name = Some("mwc-storage-ci".into());
        let mut workspace = Workspace {
            id,
            short_id: short_id.clone(),
            organization_id: Uuid::nil(),
            owner_id: Uuid::nil(),
            name: format!("Rust product storage CI BuildKit {buildkit}"),
            template_id: None,
            node_pool: "default".into(),
            runtime: WorkspaceRuntimeIdentity::new(id, &short_id).unwrap(),
            home_volume_binding: None,
            scratch_volume_binding: None,
            template,
            state: WorkspaceState::Ready,
            generation: 1,
            created_at: 1,
            updated_at: 1,
        };
        let defaults = builder.build(&workspace).unwrap();
        assert_eq!(
            defaults
                .scratch_claim
                .unwrap()
                .spec
                .unwrap()
                .resources
                .unwrap()
                .requests
                .unwrap()["storage"]
                .0,
            "10Gi"
        );
        // One GiB stays below unchanged product ephemeral eviction thresholds.
        workspace.template.storage_policy.temporary_storage_gib = 1;
        let names = WorkspaceRuntimeNames::for_workspace(
            &builder.installation_id,
            &workspace.runtime,
            &short_id,
        )
        .unwrap();
        let resources = builder.build(&workspace).unwrap();
        let sts = resources.stateful_set;
        let spec = sts.spec.as_ref().unwrap().template.spec.as_ref().unwrap();
        assert!(!spec.host_network.unwrap_or(false));
        for container in spec
            .containers
            .iter()
            .chain(spec.init_containers.iter().flatten())
        {
            assert!(
                !container
                    .security_context
                    .as_ref()
                    .and_then(|s| s.privileged)
                    .unwrap_or(false)
            );
            for mount in container
                .volume_mounts
                .iter()
                .flatten()
                .filter(|m| m.name == "workspace-scratch")
            {
                let path = mount.sub_path.as_deref().unwrap_or("");
                assert!(
                    path == "data" || path.starts_with("data/"),
                    "{} exposes {}",
                    container.name,
                    path
                );
            }
        }
        let pod = Pod {
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                name: Some(names.resources.pod_ordinal_zero()),
                namespace: Some(names.namespace.clone()),
                ..sts
                    .spec
                    .as_ref()
                    .unwrap()
                    .template
                    .metadata
                    .clone()
                    .unwrap()
            },
            spec: Some(spec.clone()),
            ..Pod::default()
        };
        std::fs::write(directory.join(format!("buildkit-{buildkit}.json")), serde_json::to_vec_pretty(&json!({
            "namespace": resources.namespace, "statefulset": sts, "pod": pod,
            "scratch": resources.scratch_claim, "service": resources.service,
            "config": resources.workspace_config, "identity": resources.ssh_identity,
            "injections": [resources.injections.environment_secret, resources.injections.environment_config_map,
                resources.injections.file_secret, resources.injections.file_config_map],
            "names": {"pod": names.resources.pod_ordinal_zero(), "home": names.resources.data_pvc_ordinal_zero(),
                "scratch": names.resources.scratch_pvc(), "service": names.resources.service},
            "scope": {"home_gib":20,"scratch_gib":1,"production_defaults_home_gib":20,"production_defaults_scratch_gib":10,
                "ttyd_tmp":"existing Memory emptyDir", "runtime_ssh":"existing Memory emptyDir", "eviction_limits_changed":false}
        })).unwrap()).unwrap();
    }
}
