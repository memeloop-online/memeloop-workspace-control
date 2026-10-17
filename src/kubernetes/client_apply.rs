use crate::workspaces::Workspace;
use k8s_openapi::api::{
    apps::v1::StatefulSet,
    core::v1::{ConfigMap, Namespace, PersistentVolumeClaim, Pod, Secret, Service, ServiceAccount},
    networking::v1::{Ingress, NetworkPolicy},
    rbac::v1::ClusterRoleBinding,
};
use kube::{
    Api,
    api::{DeleteParams, Patch, PatchParams},
};
use serde::{Serialize, de::DeserializeOwned};
use std::{fmt::Debug, future::Future, time::Instant};
use uuid::Uuid;

use super::super::{DesiredResources, InjectionMaterialization};
use super::{
    FIELD_MANAGER, KubernetesCoordinator, ReconcileError, restart_generation_is_stale,
    verify_existing,
};

#[path = "client_apply/envoy_filter.rs"]
mod envoy_filter;
#[path = "client_apply/http_proxy.rs"]
mod http_proxy;
#[path = "client_apply/port_mappings.rs"]
mod port_mappings;
#[path = "client_apply/startup_gate.rs"]
mod startup_gate;

// Record only phase names, identity and timing; injection values and API bodies stay out of logs.
async fn observe_apply_stage<T, E>(
    workspace: &Workspace,
    phase: &'static str,
    operation: impl Future<Output = Result<T, E>>,
) -> Result<T, E> {
    let started = Instant::now();
    let result = operation.await;
    tracing::info!(
        workspace_id = %workspace.id,
        generation = workspace.generation,
        phase,
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
        success = result.is_ok(),
        "workspace resource apply stage completed"
    );
    result
}

impl KubernetesCoordinator {
    pub(super) async fn apply_desired(
        &self,
        workspace: &Workspace,
        desired: &DesiredResources,
    ) -> Result<(), ReconcileError> {
        let namespace_name = desired
            .namespace
            .metadata
            .name
            .as_deref()
            .ok_or(ReconcileError::MissingObjectName)?;
        if namespace_name != workspace.runtime.namespace() {
            return Err(ReconcileError::RuntimeIdentityMismatch);
        }
        observe_apply_stage(
            workspace,
            "home_binding",
            self.verify_home_volume_binding(workspace),
        )
        .await?;
        observe_apply_stage(
            workspace,
            "namespace",
            self.apply_namespace(workspace, desired),
        )
        .await?;
        let startup = observe_apply_stage(
            workspace,
            "startup_plan",
            Box::pin(self.prepare_startup_plan(workspace, desired)),
        )
        .await?;
        Box::pin(self.apply_startup_prerequisites(namespace_name, workspace, desired, &startup))
            .await?;
        // A new Pod may prepare its volumes while injection writes are pending, but its
        // required startup ConfigMap keeps every container blocked until all inputs and routes
        // succeed. Existing workspaces keep the original update ordering.
        observe_apply_stage(
            workspace,
            "network",
            self.apply_network(namespace_name, workspace, desired),
        )
        .await?;
        if let Some(gate) = &startup.gate {
            let config_maps = Api::<ConfigMap>::namespaced(self.client.clone(), namespace_name);
            let name = gate
                .metadata
                .name
                .as_deref()
                .ok_or(ReconcileError::MissingObjectName)?;
            observe_apply_stage(
                workspace,
                "startup_approval",
                self.apply_owned(&config_maps, name, gate, workspace.id),
            )
            .await?;
        }
        if !startup.overlap {
            observe_apply_stage(
                workspace,
                "workload",
                Box::pin(self.apply_workload(namespace_name, workspace, &startup.workload)),
            )
            .await?;
        }
        Ok(())
    }

    async fn apply_namespace(
        &self,
        workspace: &Workspace,
        desired: &DesiredResources,
    ) -> Result<(), ReconcileError> {
        let namespace_name = workspace.runtime.namespace();
        let namespaces = Api::<Namespace>::all(self.client.clone());
        if let Some(existing) = namespaces.get_metadata_opt(namespace_name).await? {
            self.builder
                .verify_installation_ownership(&existing.metadata)?;
            if let Some(actual) = existing
                .metadata
                .labels
                .as_ref()
                .and_then(|labels| labels.get(super::super::WORKSPACE_ID_LABEL))
            {
                return Err(super::super::OwnershipError::LabelMismatch {
                    key: super::super::WORKSPACE_ID_LABEL,
                    expected: "absent from the product namespace".to_owned(),
                    actual: Some(actual.clone()),
                }
                .into());
            }
        }
        namespaces
            .patch(
                namespace_name,
                &PatchParams::apply(FIELD_MANAGER),
                &Patch::Apply(&desired.namespace),
            )
            .await?;
        Ok(())
    }

    async fn apply_access_identity(
        &self,
        namespace_name: &str,
        workspace: &Workspace,
        desired: &DesiredResources,
    ) -> Result<(), ReconcileError> {
        let workspace_id = workspace.id;
        let apply = PatchParams::apply(FIELD_MANAGER);
        let runtime_names = self.builder.runtime_names(workspace)?;
        let names = &runtime_names.resources;
        let service_accounts =
            Api::<ServiceAccount>::namespaced(self.client.clone(), namespace_name);
        let cluster_role_bindings = Api::<ClusterRoleBinding>::all(self.client.clone());
        let binding_name = self.builder.cluster_admin_binding_name(workspace)?;
        if let Some(service_account) = &desired.service_account {
            verify_existing(
                &service_accounts,
                &names.service_account,
                &self.builder,
                workspace_id,
            )
            .await?;
            service_accounts
                .patch(
                    &names.service_account,
                    &apply,
                    &Patch::Apply(service_account),
                )
                .await?;
        }
        if let Some(binding) = &desired.cluster_role_binding {
            verify_existing(
                &cluster_role_bindings,
                &binding_name,
                &self.builder,
                workspace_id,
            )
            .await?;
            cluster_role_bindings
                .patch(&binding_name, &apply, &Patch::Apply(binding))
                .await?;
        } else {
            if let Some(existing) = cluster_role_bindings.get_opt(&binding_name).await? {
                self.builder
                    .verify_delete_ownership(&existing.metadata, workspace_id)?;
                cluster_role_bindings
                    .delete(&binding_name, &DeleteParams::default())
                    .await?;
            }
            if let Some(existing) = service_accounts.get_opt(&names.service_account).await? {
                self.builder
                    .verify_delete_ownership(&existing.metadata, workspace_id)?;
                service_accounts
                    .delete(&names.service_account, &DeleteParams::default())
                    .await?;
            }
        }
        Ok(())
    }

    async fn apply_workspace_identity(
        &self,
        namespace_name: &str,
        workspace: &Workspace,
        desired: &DesiredResources,
    ) -> Result<(), ReconcileError> {
        let workspace_id = workspace.id;
        let runtime_names = self.builder.runtime_names(workspace)?;
        let names = &runtime_names.resources;
        let secrets = Api::<Secret>::namespaced(self.client.clone(), namespace_name);
        let config_maps = Api::<ConfigMap>::namespaced(self.client.clone(), namespace_name);
        tokio::try_join!(
            self.apply_owned(
                &secrets,
                &names.ssh_identity_secret,
                &desired.ssh_identity,
                workspace_id,
            ),
            self.apply_owned(
                &config_maps,
                &names.workspace_config,
                &desired.workspace_config,
                workspace_id,
            ),
        )?;
        Ok(())
    }

    async fn apply_workspace_services(
        &self,
        namespace_name: &str,
        workspace: &Workspace,
        desired: &DesiredResources,
    ) -> Result<(), ReconcileError> {
        let workspace_id = workspace.id;
        let runtime_names = self.builder.runtime_names(workspace)?;
        let names = &runtime_names.resources;
        let apply = PatchParams::apply(FIELD_MANAGER);
        let services = Api::<Service>::namespaced(self.client.clone(), namespace_name);
        observe_apply_stage(
            workspace,
            "service",
            self.apply_owned(&services, &names.service, &desired.service, workspace_id),
        )
        .await?;
        if let Some(service) = &desired.internal_ssh_service {
            verify_existing(&services, &names.ssh_service, &self.builder, workspace_id).await?;
            services
                .patch(&names.ssh_service, &apply, &Patch::Apply(service))
                .await?;
        } else if let Some(existing) = services.get_opt(&names.ssh_service).await? {
            self.builder
                .verify_delete_ownership(&existing.metadata, workspace_id)?;
            services
                .delete(&names.ssh_service, &DeleteParams::default())
                .await?;
        }
        Ok(())
    }

    async fn apply_workload(
        &self,
        namespace_name: &str,
        workspace: &Workspace,
        desired: &StatefulSet,
    ) -> Result<(), ReconcileError> {
        let workspace_id = workspace.id;
        let runtime_names = self.builder.runtime_names(workspace)?;
        let names = &runtime_names.resources;
        let apply = PatchParams::apply(FIELD_MANAGER);
        let stateful_sets = Api::<StatefulSet>::namespaced(self.client.clone(), namespace_name);
        let mut stateful_set = desired.clone();
        if let Some(existing) = stateful_sets.get_opt(&names.stateful_set).await? {
            self.builder
                .verify_delete_ownership(&existing.metadata, workspace_id)?;
            verify_unpersisted_home(
                workspace,
                &existing,
                &names.data_claim_template,
                &names.data_pvc_ordinal_zero(),
            )?;
            if existing.metadata.deletion_timestamp.is_some() {
                return Err(ReconcileError::StatefulSetRecreationPending);
            }
            preserve_claim_storage_classes(&mut stateful_set, &existing);
        }
        self.verify_home_volume_binding(workspace).await?;
        let stateful_set_result = observe_apply_stage(
            workspace,
            "stateful_set",
            stateful_sets.patch(&names.stateful_set, &apply, &Patch::Apply(&stateful_set)),
        )
        .await;
        if let Err(error) = stateful_set_result {
            if immutable_stateful_set_update(&error) {
                self.verify_home_volume_binding(workspace).await?;
                stateful_sets
                    .delete(&names.stateful_set, &DeleteParams::default())
                    .await?;
                return Err(ReconcileError::StatefulSetRecreationPending);
            }
            return Err(error.into());
        }
        let persistent_volume_claims =
            Api::<PersistentVolumeClaim>::namespaced(self.client.clone(), namespace_name);
        let pvc_name = names.data_pvc_ordinal_zero();
        if workspace.home_volume_binding.is_none()
            && let Some(existing) = persistent_volume_claims.get_opt(&pvc_name).await?
        {
            self.builder
                .verify_delete_ownership(&existing.metadata, workspace_id)?;
            let claim_labels = desired
                .spec
                .as_ref()
                .and_then(|spec| spec.volume_claim_templates.as_ref())
                .and_then(|claims| claims.first())
                .and_then(|claim| claim.metadata.labels.as_ref());
            if claim_labels_need_update(&existing, claim_labels) {
                // StatefulSet-created claims already carry these labels. Avoid an unnecessary
                // write while the scheduler is updating the claim during VolumeBinding.
                persistent_volume_claims
                    .patch(
                        &pvc_name,
                        &PatchParams::default(),
                        &Patch::Merge(&serde_json::json!({
                            "metadata": {"labels": claim_labels}
                        })),
                    )
                    .await?;
            }
        }
        if matches!(
            workspace.state,
            crate::workspaces::WorkspaceState::Starting
                | crate::workspaces::WorkspaceState::Restarting
        ) {
            let pods = Api::<Pod>::namespaced(self.client.clone(), namespace_name);
            let pod_name = names.pod_ordinal_zero();
            if let Some(pod) = pods.get_opt(&pod_name).await? {
                self.builder
                    .verify_delete_ownership(&pod.metadata, workspace_id)?;
                if restart_generation_is_stale(&pod, workspace.generation) {
                    pods.delete(&pod_name, &DeleteParams::default()).await?;
                }
            }
        }
        Ok(())
    }

    async fn apply_network_policy(
        &self,
        namespace_name: &str,
        workspace: &Workspace,
        desired: &DesiredResources,
    ) -> Result<(), ReconcileError> {
        let workspace_id = workspace.id;
        let apply = PatchParams::apply(FIELD_MANAGER);
        let runtime_names = self.builder.runtime_names(workspace)?;
        let names = &runtime_names.resources;
        let network_policies =
            Api::<NetworkPolicy>::namespaced(self.client.clone(), namespace_name);
        verify_existing(
            &network_policies,
            &names.network_policy,
            &self.builder,
            workspace_id,
        )
        .await?;
        network_policies
            .patch(
                &names.network_policy,
                &apply,
                &Patch::Apply(&desired.network_policy),
            )
            .await?;
        Ok(())
    }

    async fn apply_network(
        &self,
        namespace_name: &str,
        workspace: &Workspace,
        desired: &DesiredResources,
    ) -> Result<(), ReconcileError> {
        let workspace_id = workspace.id;
        let apply = PatchParams::apply(FIELD_MANAGER);
        let runtime_names = self.builder.runtime_names(workspace)?;
        let names = &runtime_names.resources;
        if self.builder.ttyd_mtls.is_some() && desired.web_shell_ingress.is_some() {
            self.apply_web_shell_envoy_filter(workspace, desired)
                .await?;
        }
        let ingresses = Api::<Ingress>::namespaced(self.client.clone(), namespace_name);
        if let Some(existing) = ingresses.get_opt(&names.web_shell_ingress).await? {
            self.builder
                .verify_delete_ownership(&existing.metadata, workspace_id)?;
            if desired.web_shell_ingress.is_none() {
                ingresses
                    .delete(&names.web_shell_ingress, &DeleteParams::default())
                    .await?;
                if self.builder.ttyd_mtls.is_some() {
                    // DELETE acceptance is not absence: retain the SAN protection until
                    // a subsequent reconciliation observes the route has disappeared.
                    return Err(ReconcileError::WebShellIngressTerminating);
                }
            }
        }
        if let Some(ingress) = &desired.web_shell_ingress {
            ingresses
                .patch(&names.web_shell_ingress, &apply, &Patch::Apply(ingress))
                .await?;
        }
        if self.builder.ttyd_mtls.is_some() && desired.web_shell_ingress.is_none() {
            self.apply_web_shell_envoy_filter(workspace, desired)
                .await?;
        }
        Ok(())
    }

    async fn apply_injections(
        &self,
        workspace: &Workspace,
        materialization: &InjectionMaterialization,
    ) -> Result<(), ReconcileError> {
        let workspace_id = workspace.id;
        let runtime_names = self.builder.runtime_names(workspace)?;
        let names = &runtime_names.resources;
        let namespace = materialization
            .file_config_map
            .metadata
            .namespace
            .as_deref()
            .ok_or(ReconcileError::MissingObjectName)?;
        if namespace != workspace.runtime.namespace() {
            return Err(ReconcileError::RuntimeIdentityMismatch);
        }
        let secrets = Api::<Secret>::namespaced(self.client.clone(), namespace);
        let config_maps = Api::<ConfigMap>::namespaced(self.client.clone(), namespace);
        // These projected inputs are independent. Join all writes before startup approval
        // or updates to existing workloads, retaining the ownership check for every object.
        tokio::try_join!(
            self.apply_owned(
                &secrets,
                &names.environment_secret,
                &materialization.environment_secret,
                workspace_id,
            ),
            self.apply_owned(
                &secrets,
                &names.files_secret,
                &materialization.file_secret,
                workspace_id,
            ),
            self.apply_owned(
                &config_maps,
                &names.environment_config_map,
                &materialization.environment_config_map,
                workspace_id,
            ),
            self.apply_owned(
                &config_maps,
                &names.files_config_map,
                &materialization.file_config_map,
                workspace_id,
            ),
        )?;
        Ok(())
    }

    async fn apply_owned<K>(
        &self,
        api: &Api<K>,
        name: &str,
        desired: &K,
        workspace_id: Uuid,
    ) -> Result<(), ReconcileError>
    where
        K: Clone + Debug + DeserializeOwned + Serialize,
    {
        verify_existing(api, name, &self.builder, workspace_id).await?;
        api.patch(
            name,
            &PatchParams::apply(FIELD_MANAGER),
            &Patch::Apply(desired),
        )
        .await?;
        Ok(())
    }
}

// A changed installation default applies only when provisioning a new Home.
// Updating an existing claim template would force an unnecessary Pod recreation.
fn claim_labels_need_update(
    existing: &PersistentVolumeClaim,
    desired: Option<&std::collections::BTreeMap<String, String>>,
) -> bool {
    desired.is_some_and(|labels| {
        labels.iter().any(|(key, value)| {
            existing
                .metadata
                .labels
                .as_ref()
                .and_then(|current| current.get(key))
                != Some(value)
        })
    })
}

fn preserve_claim_storage_classes(desired: &mut StatefulSet, existing: &StatefulSet) {
    let Some(claims) = desired
        .spec
        .as_mut()
        .and_then(|spec| spec.volume_claim_templates.as_mut())
    else {
        return;
    };
    let existing_claims = existing
        .spec
        .as_ref()
        .and_then(|spec| spec.volume_claim_templates.as_ref());
    for claim in claims {
        if let Some(existing) = existing_claims.into_iter().flatten().find(|existing| {
            claim.metadata.name.is_some() && existing.metadata.name == claim.metadata.name
        }) && let Some(spec) = claim.spec.as_mut()
        {
            spec.storage_class_name = existing
                .spec
                .as_ref()
                .and_then(|spec| spec.storage_class_name.clone());
        }
    }
}

fn immutable_stateful_set_update(error: &kube::Error) -> bool {
    let kube::Error::Api(response) = error else {
        return false;
    };
    immutable_stateful_set_status(response.code, &response.reason, &response.message)
}

fn verify_unpersisted_home(
    workspace: &Workspace,
    existing: &StatefulSet,
    volume_name: &str,
    default_claim_name: &str,
) -> Result<(), ReconcileError> {
    if workspace.home_volume_binding.is_none()
        && let Some(claim) = existing
            .spec
            .as_ref()
            .and_then(|spec| spec.template.spec.as_ref())
            .and_then(|spec| spec.volumes.as_ref())
            .into_iter()
            .flatten()
            .filter(|volume| volume.name == volume_name)
            .filter_map(|volume| volume.persistent_volume_claim.as_ref())
            .find(|claim| claim.claim_name != default_claim_name)
    {
        return Err(ReconcileError::UnpersistedHomeVolumeBinding {
            claim_name: claim.claim_name.clone(),
        });
    }
    Ok(())
}

fn immutable_stateful_set_status(code: u16, reason: &str, message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    code == 422
        && reason == "Invalid"
        && message.contains("updates to statefulset spec")
        && message.contains("forbidden")
}

#[cfg(test)]
mod immutable_stateful_set_tests {
    use super::{
        claim_labels_need_update, immutable_stateful_set_status, preserve_claim_storage_classes,
    };
    use k8s_openapi::api::{
        apps::v1::{StatefulSet, StatefulSetSpec},
        core::v1::{PersistentVolumeClaim, PersistentVolumeClaimSpec},
    };

    fn workload(class: Option<&str>, claim_name: &str) -> StatefulSet {
        StatefulSet {
            spec: Some(StatefulSetSpec {
                replicas: Some(1),
                volume_claim_templates: Some(vec![PersistentVolumeClaim {
                    metadata: kube::api::ObjectMeta {
                        name: Some(claim_name.to_owned()),
                        ..Default::default()
                    },
                    spec: Some(PersistentVolumeClaimSpec {
                        storage_class_name: class.map(str::to_owned),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn matching_claim_labels_do_not_write_during_volume_binding() {
        let desired =
            std::collections::BTreeMap::from([("owner".to_owned(), "workspace".to_owned())]);
        let mut existing = PersistentVolumeClaim::default();
        assert!(claim_labels_need_update(&existing, Some(&desired)));
        existing.metadata.labels = Some(desired.clone());
        existing
            .metadata
            .labels
            .as_mut()
            .unwrap()
            .insert("scheduler".to_owned(), "extra".to_owned());
        assert!(!claim_labels_need_update(&existing, Some(&desired)));
        assert!(!claim_labels_need_update(&existing, None));
        existing
            .metadata
            .labels
            .as_mut()
            .unwrap()
            .insert("owner".to_owned(), "stale".to_owned());
        assert!(claim_labels_need_update(&existing, Some(&desired)));
    }

    #[test]
    fn changed_default_preserves_an_existing_home_class_and_other_updates() {
        for original in [Some("original-home"), None] {
            let existing = workload(original, "workspace-data");
            let mut desired = workload(Some("new-default"), "workspace-data");
            desired.spec.as_mut().unwrap().replicas = Some(0);
            preserve_claim_storage_classes(&mut desired, &existing);
            let spec = desired.spec.unwrap();
            assert_eq!(spec.replicas, Some(0));
            assert_eq!(
                spec.volume_claim_templates.unwrap()[0]
                    .spec
                    .as_ref()
                    .unwrap()
                    .storage_class_name
                    .as_deref(),
                original
            );
        }
    }

    #[test]
    fn newly_introduced_claim_keeps_the_new_default() {
        let existing = workload(Some("original-home"), "another-claim");
        let mut desired = workload(Some("new-default"), "workspace-data");
        preserve_claim_storage_classes(&mut desired, &existing);
        assert_eq!(
            desired.spec.unwrap().volume_claim_templates.unwrap()[0]
                .spec
                .as_ref()
                .unwrap()
                .storage_class_name
                .as_deref(),
            Some("new-default")
        );
    }

    #[test]
    fn recognizes_only_immutable_stateful_set_updates() {
        assert!(immutable_stateful_set_status(
            422,
            "Invalid",
            "StatefulSet.apps example is invalid: spec: Forbidden: updates to statefulset spec are forbidden",
        ));
        assert!(!immutable_stateful_set_status(
            422,
            "Invalid",
            "StatefulSet.apps example has an invalid selector",
        ));
        assert!(!immutable_stateful_set_status(
            409,
            "Conflict",
            "updates to statefulset spec are forbidden",
        ));
    }
}
