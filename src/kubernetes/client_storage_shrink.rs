use std::collections::BTreeMap;

use k8s_openapi::{
    api::{
        apps::v1::StatefulSet,
        core::v1::{PersistentVolume, PersistentVolumeClaim, Pod, PodStatus},
        networking::v1::NetworkPolicy,
        storage::v1::StorageClass,
    },
    apimachinery::pkg::apis::meta::v1::ObjectMeta,
};
use kube::core::{ApiResource, DynamicObject, GroupVersionKind};
use kube::{
    Api,
    api::{DeleteParams, ListParams, Patch, PatchParams, PostParams, Preconditions},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::super::{OWNER_INSTALLATION_LABEL, STORAGE_ROLE_LABEL, WORKSPACE_ID_LABEL};
use super::{KubernetesCoordinator, home_volume_capacity_bytes};
use crate::{
    storage::{StorageShrinkOperation, StorageShrinkVolume, StorageVolumeIdentity},
    workspaces::{Workspace, WorkspaceHomeVolumeBinding},
};

const OPERATION_LABEL: &str = "workspace.memeloop.dev/storage-shrink";
const COMPONENT_LABEL: &str = "app.kubernetes.io/component";

#[derive(Clone, Copy)]
pub(crate) enum ShrinkPodStage {
    Preflight,
    Measure,
    Copy,
    Verify,
}

impl ShrinkPodStage {
    fn name(self) -> &'static str {
        match self {
            Self::Preflight => "preflight",
            Self::Measure => "measure",
            Self::Copy => "copy",
            Self::Verify => "verify",
        }
    }
    fn script(self) -> &'static str {
        match self {
            Self::Preflight => include_str!("storage_shrink/preflight.sh"),
            Self::Measure => include_str!("storage_shrink/measure.sh"),
            Self::Copy => include_str!("storage_shrink/copy.sh"),
            Self::Verify => include_str!("storage_shrink/verify.sh"),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ShrinkUsage {
    pub used_bytes: u64,
    pub used_inodes: u64,
}

impl KubernetesCoordinator {
    pub(super) async fn verify_scratch_volume_binding(
        &self,
        workspace: &Workspace,
    ) -> Result<(), super::ReconcileError> {
        if let Some(binding) = workspace.scratch_volume_binding.as_ref() {
            self.shrink_claim(workspace, binding, true)
                .await
                .map_err(|_| super::ReconcileError::InvalidScratchVolumeBinding {
                    claim_name: binding.claim_name.clone(),
                })?;
        }
        Ok(())
    }
    pub(crate) async fn shrink_volume_identity(
        &self,
        workspace: &Workspace,
        binding: &WorkspaceHomeVolumeBinding,
    ) -> Result<StorageVolumeIdentity, String> {
        let claim = self.shrink_claim(workspace, binding, true).await?;
        let claim_spec = claim.spec.as_ref().ok_or("claim spec is missing")?;
        let pv_name = claim_spec
            .volume_name
            .as_ref()
            .ok_or("source PV name is missing")?;
        let volumes = Api::<PersistentVolume>::all(self.client.clone());
        let volume = volumes.get(pv_name).await.map_err(safe_error)?;
        let spec = volume.spec.as_ref().ok_or("PV spec is missing")?;
        let csi = spec
            .csi
            .as_ref()
            .ok_or("safe shrink/reclaim requires a CSI volume")?;
        let class_name = claim_spec
            .storage_class_name
            .as_ref()
            .ok_or("CSI storage class is missing")?;
        let classes = Api::<StorageClass>::all(self.client.clone());
        let class = classes.get(class_name).await.map_err(safe_error)?;
        if class.provisioner != csi.driver || spec.storage_class_name.as_ref() != Some(class_name) {
            return Err("CSI driver does not match the source storage class".to_owned());
        }
        let mut identity = StorageVolumeIdentity {
            pv_name: pv_name.clone(),
            pv_uid: volume.metadata.uid.clone().ok_or("PV UID is missing")?,
            csi_driver: csi.driver.clone(),
            csi_volume_handle: csi.volume_handle.clone(),
            storage_class_name: class_name.clone(),
            longhorn_namespace: None,
            longhorn_volume_uid: None,
        };
        validate_pv_identity(&volume, binding, &identity)?;
        if csi.driver == "driver.longhorn.io" {
            let namespace = std::env::var("MWC_LONGHORN_NAMESPACE")
                .unwrap_or_else(|_| "longhorn-system".to_owned());
            let resource = longhorn_resource();
            let backend =
                Api::<DynamicObject>::namespaced_with(self.client.clone(), &namespace, &resource);
            let longhorn = backend.get(&csi.volume_handle).await.map_err(safe_error)?;
            identity.longhorn_volume_uid = Some(
                longhorn
                    .metadata
                    .uid
                    .ok_or("Longhorn Volume UID is missing")?,
            );
            identity.longhorn_namespace = Some(namespace);
        }
        Ok(identity)
    }

    async fn delete_shrink_claim(
        &self,
        workspace: &Workspace,
        binding: &WorkspaceHomeVolumeBinding,
        identity: &StorageVolumeIdentity,
    ) -> Result<bool, String> {
        if binding.namespace != workspace.runtime.namespace() || !binding.is_valid() {
            return Err("invalid reclaim namespace or binding".to_owned());
        }
        let claims = Api::<PersistentVolumeClaim>::namespaced(
            self.client.clone(),
            workspace.runtime.namespace(),
        );
        let volumes = Api::<PersistentVolume>::all(self.client.clone());
        let claim = claims
            .get_opt(&binding.claim_name)
            .await
            .map_err(safe_error)?;
        let volume = volumes
            .get_opt(&identity.pv_name)
            .await
            .map_err(safe_error)?;
        if let Some(volume) = volume.as_ref() {
            validate_pv_identity(volume, binding, identity)?;
        }
        if let Some(waiting) = self
            .check_longhorn_backend(identity, &claim, &volume)
            .await?
        {
            return Ok(waiting);
        }
        let Some(claim) = claim else {
            if let Some(volume) = volume.as_ref() {
                self.delete_released_volume(&volumes, volume, identity)
                    .await?;
                return Ok(false);
            }
            return Ok(true);
        };
        if claim.metadata.uid.as_deref() != Some(binding.claim_uid.as_str()) {
            return Err("source claim UID changed; refusing reclamation".to_owned());
        }
        if claim.metadata.deletion_timestamp.is_some() {
            return Ok(false);
        }
        self.shrink_owned(&claim.metadata, workspace)?;
        self.ensure_claim_unmounted(workspace, binding).await?;
        let volume = volume
            .as_ref()
            .ok_or("source claim's verified PV disappeared before reclamation")?;
        if volume
            .spec
            .as_ref()
            .and_then(|spec| spec.persistent_volume_reclaim_policy.as_deref())
            != Some("Delete")
        {
            volumes.patch(&identity.pv_name, &PatchParams::default(), &Patch::Merge(&json!({"metadata":{"uid":identity.pv_uid,"resourceVersion":volume.metadata.resource_version},"spec":{"persistentVolumeReclaimPolicy":"Delete"}}))).await.map_err(safe_error)?;
            return Ok(false);
        }
        claims
            .delete(
                &binding.claim_name,
                &DeleteParams {
                    preconditions: Some(Preconditions {
                        uid: Some(binding.claim_uid.clone()),
                        resource_version: claim.metadata.resource_version,
                    }),
                    ..DeleteParams::default()
                },
            )
            .await
            .map_err(safe_error)?;
        Ok(false)
    }

    async fn check_longhorn_backend(
        &self,
        identity: &StorageVolumeIdentity,
        claim: &Option<PersistentVolumeClaim>,
        volume: &Option<PersistentVolume>,
    ) -> Result<Option<bool>, String> {
        if let (Some(namespace), Some(expected_uid)) =
            (&identity.longhorn_namespace, &identity.longhorn_volume_uid)
        {
            let backend = Api::<DynamicObject>::namespaced_with(
                self.client.clone(),
                namespace,
                &longhorn_resource(),
            );
            if let Some(longhorn) = backend
                .get_opt(&identity.csi_volume_handle)
                .await
                .map_err(safe_error)?
            {
                if longhorn.metadata.uid.as_ref() != Some(expected_uid) {
                    return Err("Longhorn Volume UID changed; refusing reclamation".to_owned());
                }
                if claim.is_none() && volume.is_none() {
                    return Ok(Some(false));
                }
            }
        } else if identity.csi_driver == "driver.longhorn.io" {
            return Err(
                "Longhorn backend identity was not recorded; refusing reclamation".to_owned(),
            );
        }
        Ok(None)
    }

    async fn delete_released_volume(
        &self,
        volumes: &Api<PersistentVolume>,
        volume: &PersistentVolume,
        identity: &StorageVolumeIdentity,
    ) -> Result<(), String> {
        if volume
            .spec
            .as_ref()
            .and_then(|spec| spec.persistent_volume_reclaim_policy.as_deref())
            != Some("Delete")
        {
            volumes.patch(&identity.pv_name, &PatchParams::default(), &Patch::Merge(&json!({"metadata":{"uid":identity.pv_uid,"resourceVersion":volume.metadata.resource_version},"spec":{"persistentVolumeReclaimPolicy":"Delete"}}))).await.map_err(safe_error)?;
        } else if volume
            .status
            .as_ref()
            .and_then(|status| status.phase.as_deref())
            .is_some_and(|phase| matches!(phase, "Released" | "Failed"))
            && volume.metadata.deletion_timestamp.is_none()
        {
            volumes
                .delete(
                    &identity.pv_name,
                    &DeleteParams {
                        preconditions: Some(Preconditions {
                            uid: Some(identity.pv_uid.clone()),
                            resource_version: volume.metadata.resource_version.clone(),
                        }),
                        ..DeleteParams::default()
                    },
                )
                .await
                .map_err(safe_error)?;
        }
        Ok(())
    }

    async fn ensure_claim_unmounted(
        &self,
        workspace: &Workspace,
        binding: &WorkspaceHomeVolumeBinding,
    ) -> Result<(), String> {
        let pods = Api::<Pod>::namespaced(self.client.clone(), workspace.runtime.namespace());
        for pod in pods
            .list(&ListParams::default())
            .await
            .map_err(safe_error)?
            .items
        {
            if pod.spec.as_ref().is_some_and(|spec| {
                spec.volumes.iter().flatten().any(|volume| {
                    volume
                        .persistent_volume_claim
                        .as_ref()
                        .is_some_and(|claim| claim.claim_name == binding.claim_name)
                })
            }) {
                return Err("retained claim is still mounted; refusing reclamation".to_owned());
            }
        }
        Ok(())
    }

    pub(crate) async fn reclaim_shrink_sources(
        &self,
        workspace: &Workspace,
        operation: &StorageShrinkOperation,
    ) -> Result<bool, String> {
        let mut complete = true;
        for (selected, current) in [
            (&operation.home, workspace.home_volume_binding.as_ref()),
            (
                &operation.temporary,
                workspace.scratch_volume_binding.as_ref(),
            ),
        ] {
            if selected.target_gib.is_none() {
                continue;
            }
            if !selected.verified || selected.destination.is_none() {
                return Err(
                    "reclamation requires checksum-verified destination bindings".to_owned(),
                );
            }
            let destination = selected
                .destination
                .as_ref()
                .ok_or("destination is missing")?;
            if current != Some(destination) {
                return Err(
                    "workspace no longer references the verified destination; refusing reclamation"
                        .to_owned(),
                );
            }
            self.shrink_claim(workspace, destination, true).await?;
            let source = selected.source.as_ref().ok_or("source is missing")?;
            if source.claim_uid == destination.claim_uid {
                return Err("source and destination UID must differ".to_owned());
            }
            let identity = selected
                .source_volume
                .as_ref()
                .ok_or("source PV/CSI identity was not recorded")?;
            complete &= self
                .delete_shrink_claim(workspace, source, identity)
                .await?;
        }
        Ok(complete)
    }

    pub(crate) async fn capture_shrink_cleanup_identities(
        &self,
        workspace: &Workspace,
        operation: &mut StorageShrinkOperation,
    ) -> Result<(), String> {
        let claims = Api::<PersistentVolumeClaim>::namespaced(
            self.client.clone(),
            workspace.runtime.namespace(),
        );
        for (role, selected) in [
            ("home", &mut operation.home),
            ("temporary", &mut operation.temporary),
        ] {
            if let Some(target_gib) = selected.target_gib
                && selected.destination.is_none()
            {
                let name = format!("shrink-{}-{role}", operation.id.simple());
                if let Some(claim) = claims.get_opt(&name).await.map_err(safe_error)? {
                    self.shrink_owned(&claim.metadata, workspace)?;
                    let labels = claim
                        .metadata
                        .labels
                        .as_ref()
                        .ok_or("destination labels missing")?;
                    if labels.get(OPERATION_LABEL) != Some(&operation.id.to_string())
                        || labels.get(STORAGE_ROLE_LABEL).map(String::as_str) != Some(role)
                        || claim
                            .metadata
                            .owner_references
                            .as_ref()
                            .is_some_and(|owners| !owners.is_empty())
                    {
                        return Err("unrecorded destination has invalid ownership".to_owned());
                    }
                    let binding = WorkspaceHomeVolumeBinding {
                        namespace: workspace.runtime.namespace().to_owned(),
                        claim_name: name,
                        claim_uid: claim
                            .metadata
                            .uid
                            .clone()
                            .ok_or("destination UID is missing")?,
                        capacity_gib: target_gib,
                    };
                    self.shrink_claim(workspace, &binding, false).await?;
                    selected.destination = Some(binding);
                }
            }
            for (binding, identity, destination) in [
                (selected.source.as_ref(), &mut selected.source_volume, false),
                (
                    selected.destination.as_ref(),
                    &mut selected.destination_volume,
                    true,
                ),
            ] {
                let Some(binding) = binding else {
                    continue;
                };
                if identity.is_some() {
                    continue;
                }
                let Some(claim) = claims
                    .get_opt(&binding.claim_name)
                    .await
                    .map_err(safe_error)?
                else {
                    continue;
                };
                self.shrink_owned(&claim.metadata, workspace)?;
                if claim.metadata.uid.as_ref() != Some(&binding.claim_uid)
                    || destination
                        && claim
                            .metadata
                            .labels
                            .as_ref()
                            .and_then(|labels| labels.get(OPERATION_LABEL))
                            != Some(&operation.id.to_string())
                {
                    return Err("storage cleanup claim identity changed".to_owned());
                }
                if claim
                    .spec
                    .as_ref()
                    .and_then(|spec| spec.volume_name.as_ref())
                    .is_some()
                {
                    *identity = Some(self.shrink_volume_identity(workspace, binding).await?);
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn delete_workspace_shrink_storage(
        &self,
        workspace: &Workspace,
        operations: &[StorageShrinkOperation],
    ) -> Result<bool, String> {
        let mut complete = true;
        let mut seen = std::collections::BTreeSet::new();
        for operation in operations {
            for (binding, identity) in [
                (
                    operation.home.source.as_ref(),
                    operation.home.source_volume.as_ref(),
                ),
                (
                    operation.home.destination.as_ref(),
                    operation.home.destination_volume.as_ref(),
                ),
                (
                    operation.temporary.source.as_ref(),
                    operation.temporary.source_volume.as_ref(),
                ),
                (
                    operation.temporary.destination.as_ref(),
                    operation.temporary.destination_volume.as_ref(),
                ),
            ] {
                if let (Some(binding), Some(identity)) = (binding, identity)
                    && seen.insert(binding.claim_uid.clone())
                {
                    complete &= self
                        .delete_shrink_claim(workspace, binding, identity)
                        .await?;
                }
            }
            for selected in [&operation.home, &operation.temporary] {
                if selected.destination_volume.is_some() {
                    continue;
                }
                let Some(binding) = selected.destination.as_ref() else {
                    continue;
                };
                let claims = Api::<PersistentVolumeClaim>::namespaced(
                    self.client.clone(),
                    workspace.runtime.namespace(),
                );
                let Some(claim) = claims
                    .get_opt(&binding.claim_name)
                    .await
                    .map_err(safe_error)?
                else {
                    continue;
                };
                if claim.metadata.deletion_timestamp.is_some() {
                    complete = false;
                    continue;
                }
                self.shrink_owned(&claim.metadata, workspace)?;
                if claim.metadata.uid.as_ref() != Some(&binding.claim_uid)
                    || claim
                        .metadata
                        .labels
                        .as_ref()
                        .and_then(|labels| labels.get(OPERATION_LABEL))
                        != Some(&operation.id.to_string())
                {
                    return Err(
                        "unverified destination ownership changed before cleanup".to_owned()
                    );
                }
                if claim
                    .spec
                    .as_ref()
                    .and_then(|spec| spec.volume_name.as_ref())
                    .is_none()
                {
                    claims
                        .delete(
                            &binding.claim_name,
                            &DeleteParams {
                                preconditions: Some(Preconditions {
                                    uid: Some(binding.claim_uid.clone()),
                                    resource_version: claim.metadata.resource_version,
                                }),
                                ..DeleteParams::default()
                            },
                        )
                        .await
                        .map_err(safe_error)?;
                    complete = false;
                } else {
                    return Err(
                        "bound destination cleanup requires a durable PV identity before deletion"
                            .to_owned(),
                    );
                }
            }
        }
        Ok(complete)
    }
    fn shrink_owned(&self, metadata: &ObjectMeta, workspace: &Workspace) -> Result<(), String> {
        let labels = metadata
            .labels
            .as_ref()
            .ok_or("storage migration requires ownership labels")?;
        if metadata.namespace.as_deref() != Some(workspace.runtime.namespace())
            || labels.get(WORKSPACE_ID_LABEL) != Some(&workspace.id.to_string())
            || labels.get(OWNER_INSTALLATION_LABEL)
                != Some(&self.builder.installation_id.to_string())
            || metadata.uid.is_none()
            || metadata.deletion_timestamp.is_some()
        {
            return Err("storage migration resource ownership/UID validation failed".to_owned());
        }
        Ok(())
    }

    async fn shrink_claim(
        &self,
        workspace: &Workspace,
        binding: &WorkspaceHomeVolumeBinding,
        bound: bool,
    ) -> Result<PersistentVolumeClaim, String> {
        if !binding.is_valid() || binding.namespace != workspace.runtime.namespace() {
            return Err("invalid storage migration claim namespace or binding".to_owned());
        }
        let claims = Api::<PersistentVolumeClaim>::namespaced(
            self.client.clone(),
            workspace.runtime.namespace(),
        );
        let claim = claims.get(&binding.claim_name).await.map_err(safe_error)?;
        self.shrink_owned(&claim.metadata, workspace)?;
        let spec = claim.spec.as_ref().ok_or("claim spec is missing")?;
        let bytes = binding
            .capacity_gib
            .checked_mul(1 << 30)
            .ok_or("claim capacity overflow")?;
        let requested = spec
            .resources
            .as_ref()
            .and_then(|resources| resources.requests.as_ref())
            .and_then(|resources| resources.get("storage"))
            .and_then(|quantity| home_volume_capacity_bytes(&quantity.0));
        let capacity = claim
            .status
            .as_ref()
            .and_then(|status| status.capacity.as_ref())
            .and_then(|capacity| capacity.get("storage"))
            .and_then(|quantity| home_volume_capacity_bytes(&quantity.0));
        if claim.metadata.uid.as_deref() != Some(binding.claim_uid.as_str())
            || spec
                .volume_mode
                .as_deref()
                .is_some_and(|mode| mode != "Filesystem")
            || requested != Some(bytes)
            || bound
                && (claim
                    .status
                    .as_ref()
                    .and_then(|status| status.phase.as_deref())
                    != Some("Bound")
                    || capacity != Some(bytes))
        {
            return Err(
                "claim UID, filesystem mode or capacity changed during migration".to_owned(),
            );
        }
        Ok(claim)
    }

    pub(crate) async fn discover_shrink_sources(
        &self,
        workspace: &Workspace,
        operation: &mut StorageShrinkOperation,
    ) -> Result<(), String> {
        let names = self
            .builder
            .runtime_names(workspace)
            .map_err(safe_error)?
            .resources;
        let claims = Api::<PersistentVolumeClaim>::namespaced(
            self.client.clone(),
            workspace.runtime.namespace(),
        );
        if operation.home.source.is_none() {
            let claim = claims
                .get(&names.data_pvc_ordinal_zero())
                .await
                .map_err(safe_error)?;
            self.shrink_owned(&claim.metadata, workspace)?;
            operation.home.source = Some(binding_from_claim(&claim, workspace)?);
        }
        let home = operation
            .home
            .source
            .as_ref()
            .ok_or("Home source claim was not found")?;
        self.shrink_claim(workspace, home, true).await?;
        operation.original_home_gib = home.capacity_gib;
        if operation.home.source_volume.is_none() {
            operation.home.source_volume =
                Some(self.shrink_volume_identity(workspace, home).await?);
        }
        if operation
            .home
            .target_gib
            .is_some_and(|target| target >= home.capacity_gib)
        {
            return Err("Home target is not smaller than actual source capacity".to_owned());
        }
        if operation.temporary.source.is_none() {
            self.discover_scratch_source(workspace, operation, &names.pod_ordinal_zero())
                .await?;
        }
        if let Some(scratch) = operation.temporary.source.as_ref() {
            self.shrink_claim(workspace, scratch, true).await?;
            operation.original_temporary_storage_gib = scratch.capacity_gib;
            if operation.temporary.source_volume.is_none() {
                operation.temporary.source_volume =
                    Some(self.shrink_volume_identity(workspace, scratch).await?);
            }
            if operation
                .temporary
                .target_gib
                .is_some_and(|target| target >= scratch.capacity_gib)
            {
                return Err(
                    "temporary target is not smaller than actual source capacity".to_owned(),
                );
            }
        }
        Ok(())
    }

    async fn discover_scratch_source(
        &self,
        workspace: &Workspace,
        operation: &mut StorageShrinkOperation,
        pod_name: &str,
    ) -> Result<(), String> {
        let claims = Api::<PersistentVolumeClaim>::namespaced(
            self.client.clone(),
            workspace.runtime.namespace(),
        );
        let pods = Api::<Pod>::namespaced(self.client.clone(), workspace.runtime.namespace());
        let Some(pod) = pods.get_opt(pod_name).await.map_err(safe_error)? else {
            if operation.temporary.target_gib.is_some() {
                return Err("stopped workspace has no retained scratch claim; start it before requesting a scratch shrink".to_owned());
            }
            return Ok(());
        };
        self.shrink_owned(&pod.metadata, workspace)?;
        let scratch = pod
            .spec
            .as_ref()
            .and_then(|spec| spec.volumes.as_ref())
            .and_then(|volumes| {
                volumes
                    .iter()
                    .find(|volume| volume.name == "workspace-scratch")
            })
            .ok_or("workspace scratch volume is missing")?;
        let claim_name = if let Some(source) = scratch.persistent_volume_claim.as_ref() {
            source.claim_name.clone()
        } else if scratch.ephemeral.is_some() {
            format!("{pod_name}-workspace-scratch")
        } else {
            return Err("safe offline shrink requires CSI scratch; node-local emptyDir cannot be preserved by stopping a Pod".to_owned());
        };
        let claim = claims.get(&claim_name).await.map_err(safe_error)?;
        self.shrink_owned(&claim.metadata, workspace)?;
        if scratch.ephemeral.is_some()
            && !claim
                .metadata
                .owner_references
                .as_ref()
                .is_some_and(|owners| {
                    owners.iter().any(|owner| {
                        owner.kind == "Pod"
                            && Some(owner.uid.as_str()) == pod.metadata.uid.as_deref()
                            && owner.name == pod_name
                    })
                })
        {
            return Err("scratch claim is not owned by the validated workspace Pod UID".to_owned());
        }
        operation.temporary.source = Some(binding_from_claim(&claim, workspace)?);
        Ok(())
    }

    pub(crate) async fn retain_shrink_sources(
        &self,
        workspace: &Workspace,
        operation: &StorageShrinkOperation,
    ) -> Result<bool, String> {
        let names = self
            .builder
            .runtime_names(workspace)
            .map_err(safe_error)?
            .resources;
        let sets =
            Api::<StatefulSet>::namespaced(self.client.clone(), workspace.runtime.namespace());
        let set = sets
            .get_opt(&names.stateful_set)
            .await
            .map_err(safe_error)?;
        if let Some(set) = &set {
            self.shrink_owned(&set.metadata, workspace)?;
            let retained = set
                .spec
                .as_ref()
                .and_then(|spec| spec.persistent_volume_claim_retention_policy.as_ref())
                .is_some_and(|policy| {
                    policy.when_scaled.as_deref() == Some("Retain")
                        && policy.when_deleted.as_deref() == Some("Retain")
                });
            if !retained {
                sets.patch(&names.stateful_set, &PatchParams::default(), &Patch::Merge(&json!({"metadata": {"uid": set.metadata.uid, "resourceVersion": set.metadata.resource_version}, "spec": {"persistentVolumeClaimRetentionPolicy": {"whenScaled":"Retain", "whenDeleted":"Retain"}}}))).await.map_err(safe_error)?;
                return Ok(false);
            }
            if set
                .status
                .as_ref()
                .and_then(|status| status.observed_generation)
                .unwrap_or(0)
                < set.metadata.generation.unwrap_or(0)
            {
                return Ok(false);
            }
        }
        let pods = Api::<Pod>::namespaced(self.client.clone(), workspace.runtime.namespace());
        let pod = pods
            .get_opt(&names.pod_ordinal_zero())
            .await
            .map_err(safe_error)?;
        if let Some(pod) = &pod {
            self.shrink_owned(&pod.metadata, workspace)?;
        }
        let claims = Api::<PersistentVolumeClaim>::namespaced(
            self.client.clone(),
            workspace.runtime.namespace(),
        );
        for binding in [
            operation.home.source.as_ref(),
            operation.temporary.source.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            let claim = self.shrink_claim(workspace, binding, true).await?;
            for owner in claim.metadata.owner_references.iter().flatten() {
                let valid_pod = owner.kind == "Pod"
                    && pod.as_ref().is_some_and(|pod| {
                        pod.metadata.uid.as_deref() == Some(owner.uid.as_str())
                            && pod.metadata.name.as_deref() == Some(owner.name.as_str())
                    });
                let valid_set = owner.kind == "StatefulSet"
                    && set.as_ref().is_some_and(|set| {
                        set.metadata.uid.as_deref() == Some(owner.uid.as_str())
                            && set.metadata.name.as_deref() == Some(owner.name.as_str())
                    });
                if !valid_pod && !valid_set {
                    return Err(
                        "source claim has an unrelated owner; refusing to change GC ownership"
                            .to_owned(),
                    );
                }
            }
            claims.patch(&binding.claim_name, &PatchParams::default(), &Patch::Merge(&json!({"metadata": {"uid": binding.claim_uid, "resourceVersion": claim.metadata.resource_version, "ownerReferences": [], "annotations": {"workspace.memeloop.dev/retained-storage-shrink": operation.id.to_string()}}}))).await.map_err(safe_error)?;
        }
        Ok(true)
    }

    pub(crate) async fn stop_for_storage_shrink(
        &self,
        workspace: &Workspace,
        operation: &StorageShrinkOperation,
    ) -> Result<bool, String> {
        let names = self
            .builder
            .runtime_names(workspace)
            .map_err(safe_error)?
            .resources;
        let sets =
            Api::<StatefulSet>::namespaced(self.client.clone(), workspace.runtime.namespace());
        if let Some(set) = sets
            .get_opt(&names.stateful_set)
            .await
            .map_err(safe_error)?
        {
            self.shrink_owned(&set.metadata, workspace)?;
            if !set
                .spec
                .as_ref()
                .and_then(|spec| spec.persistent_volume_claim_retention_policy.as_ref())
                .is_some_and(|policy| {
                    policy.when_scaled.as_deref() == Some("Retain")
                        && policy.when_deleted.as_deref() == Some("Retain")
                })
            {
                sets.patch(&names.stateful_set, &PatchParams::default(), &Patch::Merge(&json!({"metadata": {"uid": set.metadata.uid, "resourceVersion": set.metadata.resource_version}, "spec": {"persistentVolumeClaimRetentionPolicy": {"whenScaled":"Retain", "whenDeleted":"Retain"}}}))).await.map_err(safe_error)?;
                return Ok(false);
            }
            if set
                .status
                .as_ref()
                .and_then(|status| status.observed_generation)
                .unwrap_or(0)
                < set.metadata.generation.unwrap_or(0)
            {
                return Ok(false);
            }
            for binding in [
                operation.home.source.as_ref(),
                operation.temporary.source.as_ref(),
            ]
            .into_iter()
            .flatten()
            {
                let claim = self.shrink_claim(workspace, binding, true).await?;
                if claim
                    .metadata
                    .owner_references
                    .as_ref()
                    .is_some_and(|owners| !owners.is_empty())
                {
                    return Err("source claim regained a GC owner; refusing to stop".to_owned());
                }
            }
            if set.spec.as_ref().and_then(|spec| spec.replicas) != Some(0) {
                sets.patch(&names.stateful_set, &PatchParams::default(), &Patch::Merge(&json!({"metadata": {"uid": set.metadata.uid, "resourceVersion": set.metadata.resource_version}, "spec": {"replicas": 0}}))).await.map_err(safe_error)?;
                return Ok(false);
            }
        }
        self.migration_claims_detached(workspace, operation).await
    }

    async fn migration_claims_detached(
        &self,
        workspace: &Workspace,
        operation: &StorageShrinkOperation,
    ) -> Result<bool, String> {
        let pods = Api::<Pod>::namespaced(self.client.clone(), workspace.runtime.namespace());
        let list = pods
            .list(&ListParams::default())
            .await
            .map_err(safe_error)?;
        for pod in list.items {
            let is_helper = pod
                .metadata
                .labels
                .as_ref()
                .and_then(|labels| labels.get(OPERATION_LABEL))
                == Some(&operation.id.to_string());
            if is_helper {
                continue;
            }
            let owns_workspace = pod
                .metadata
                .labels
                .as_ref()
                .and_then(|labels| labels.get(WORKSPACE_ID_LABEL))
                == Some(&workspace.id.to_string());
            let mounts_source = pod.spec.as_ref().is_some_and(|spec| {
                spec.volumes.iter().flatten().any(|volume| {
                    volume
                        .persistent_volume_claim
                        .as_ref()
                        .is_some_and(|claim| {
                            [
                                operation.home.source.as_ref(),
                                operation.temporary.source.as_ref(),
                                operation.home.destination.as_ref(),
                                operation.temporary.destination.as_ref(),
                            ]
                            .into_iter()
                            .flatten()
                            .any(|binding| binding.claim_name == claim.claim_name)
                        })
                })
            });
            if mounts_source && !owns_workspace {
                return Err(
                    "another Pod mounts a migration claim; refusing offline copy".to_owned(),
                );
            }
            if owns_workspace || mounts_source {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(crate) async fn create_shrink_destination(
        &self,
        workspace: &Workspace,
        operation: &StorageShrinkOperation,
        role: &str,
        source: &WorkspaceHomeVolumeBinding,
        target_gib: u64,
    ) -> Result<WorkspaceHomeVolumeBinding, String> {
        let source_claim = self.shrink_claim(workspace, source, true).await?;
        let spec = source_claim
            .spec
            .as_ref()
            .ok_or("source claim spec is missing")?;
        let name = format!("shrink-{}-{role}", operation.id.simple());
        let claims = Api::<PersistentVolumeClaim>::namespaced(
            self.client.clone(),
            workspace.runtime.namespace(),
        );
        let mut labels = self.builder.labels(workspace.id);
        labels.insert(OPERATION_LABEL.to_owned(), operation.id.to_string());
        labels.insert(STORAGE_ROLE_LABEL.to_owned(), role.to_owned());
        let existing = match claims.get_opt(&name).await.map_err(safe_error)? {
            Some(existing) => existing,
            None => {
                let desired: PersistentVolumeClaim = serde_json::from_value(json!({"apiVersion":"v1","kind":"PersistentVolumeClaim","metadata":{"name":name,"namespace":workspace.runtime.namespace(),"labels":labels},"spec":{"accessModes":spec.access_modes,"storageClassName":spec.storage_class_name,"volumeMode":"Filesystem","resources":{"requests":{"storage":format!("{target_gib}Gi")}}}})).map_err(safe_error)?;
                claims
                    .create(&PostParams::default(), &desired)
                    .await
                    .map_err(safe_error)?
            }
        };
        self.shrink_owned(&existing.metadata, workspace)?;
        if existing
            .metadata
            .labels
            .as_ref()
            .and_then(|labels| labels.get(OPERATION_LABEL))
            != Some(&operation.id.to_string())
            || existing
                .metadata
                .owner_references
                .as_ref()
                .is_some_and(|owners| !owners.is_empty())
        {
            return Err("destination claim is not exclusive to this migration".to_owned());
        }
        let binding = WorkspaceHomeVolumeBinding {
            namespace: workspace.runtime.namespace().to_owned(),
            claim_name: name,
            claim_uid: existing
                .metadata
                .uid
                .clone()
                .ok_or("destination claim has no UID")?,
            capacity_gib: target_gib,
        };
        self.shrink_claim(workspace, &binding, false).await?;
        Ok(binding)
    }

    pub(crate) async fn shrink_pod(
        &self,
        workspace: &Workspace,
        operation: &mut StorageShrinkOperation,
        role: &str,
        stage: ShrinkPodStage,
    ) -> Result<Option<ShrinkUsage>, String> {
        let volume = if role == "home" {
            operation.home.clone()
        } else {
            operation.temporary.clone()
        };
        let source = volume
            .source
            .as_ref()
            .ok_or("migration source is missing")?;
        let image = storage_shrink_image().map_err(safe_error)?;
        self.shrink_claim(workspace, source, true).await?;
        if !matches!(stage, ShrinkPodStage::Preflight)
            && let Some(destination) = volume.destination.as_ref()
        {
            self.shrink_claim(workspace, destination, false).await?;
        }
        let pods = Api::<Pod>::namespaced(self.client.clone(), workspace.runtime.namespace());
        let name = format!("shrink-{}-{role}-{}", operation.id.simple(), stage.name());
        let mut labels = self.builder.labels(workspace.id);
        labels.insert(COMPONENT_LABEL.to_owned(), "storage-shrink".to_owned());
        labels.insert(OPERATION_LABEL.to_owned(), operation.id.to_string());
        let pod = match pods.get_opt(&name).await.map_err(safe_error)? {
            Some(pod) => pod,
            None => {
                self.ensure_helper_policy(workspace, operation, &labels)
                    .await?;
                let metadata = ObjectMeta {
                    name: Some(name.clone()),
                    namespace: Some(workspace.runtime.namespace().to_owned()),
                    labels: Some(labels),
                    ..Default::default()
                };
                let desired = migration_pod(
                    operation,
                    metadata,
                    stage,
                    source,
                    volume.destination.as_ref(),
                    &image,
                )?;
                pods.create(&PostParams::default(), &desired)
                    .await
                    .map_err(safe_error)?
            }
        };
        self.shrink_owned(&pod.metadata, workspace)?;
        if pod
            .metadata
            .labels
            .as_ref()
            .and_then(|labels| labels.get(OPERATION_LABEL))
            != Some(&operation.id.to_string())
        {
            return Err("copy Pod belongs to another operation".to_owned());
        }
        let policies =
            Api::<NetworkPolicy>::namespaced(self.client.clone(), workspace.runtime.namespace());
        let policy = policies
            .get(&format!("shrink-{}", operation.id.simple()))
            .await
            .map_err(safe_error)?;
        self.shrink_owned(&policy.metadata, workspace)?;
        validate_helper_policy(&policy, operation)?;
        let metadata = ObjectMeta {
            name: Some(name.clone()),
            namespace: Some(workspace.runtime.namespace().to_owned()),
            labels: Some(pod.metadata.labels.clone().unwrap_or_default()),
            ..Default::default()
        };
        let desired = migration_pod(
            operation,
            metadata,
            stage,
            source,
            volume.destination.as_ref(),
            &image,
        )?;
        validate_helper_spec(&pod, &desired)?;
        self.helper_pod_result(workspace, operation, role, stage, &pod, &volume, &name)
            .await
    }

    async fn helper_pod_result(
        &self,
        workspace: &Workspace,
        operation: &mut StorageShrinkOperation,
        role: &str,
        stage: ShrinkPodStage,
        pod: &Pod,
        volume: &StorageShrinkVolume,
        name: &str,
    ) -> Result<Option<ShrinkUsage>, String> {
        let uid = pod
            .metadata
            .uid
            .clone()
            .ok_or("helper Pod UID is missing")?;
        let expected_uid = operation
            .helper_pod_uids
            .entry(name.to_owned())
            .or_insert_with(|| uid.clone());
        if *expected_uid != uid {
            return Err("helper Pod UID changed; refusing its verification evidence".to_owned());
        }
        match pod
            .status
            .as_ref()
            .and_then(|status| status.phase.as_deref())
        {
            Some("Succeeded") => {
                let usage = parse_usage_evidence(pod.status.as_ref())?;
                if !matches!(stage, ShrinkPodStage::Preflight)
                    && let Some(destination) = volume.destination.as_ref()
                {
                    self.shrink_claim(workspace, destination, true).await?;
                }
                Ok(Some(usage))
            }
            Some("Failed") => Err(format!(
                "{role} {} helper failed; check approved image tools, source readability and destination capacity",
                stage.name()
            )),
            _ => Ok(None),
        }
    }

    async fn ensure_helper_policy(
        &self,
        workspace: &Workspace,
        operation: &StorageShrinkOperation,
        labels: &BTreeMap<String, String>,
    ) -> Result<(), String> {
        let policies =
            Api::<NetworkPolicy>::namespaced(self.client.clone(), workspace.runtime.namespace());
        let policy_name = format!("shrink-{}", operation.id.simple());
        if let Some(policy) = policies.get_opt(&policy_name).await.map_err(safe_error)? {
            self.shrink_owned(&policy.metadata, workspace)?;
            if policy
                .metadata
                .labels
                .as_ref()
                .and_then(|labels| labels.get(OPERATION_LABEL))
                != Some(&operation.id.to_string())
            {
                return Err("migration network policy ownership changed".to_owned());
            }
            validate_helper_policy(&policy, operation)?;
        } else {
            let policy: NetworkPolicy = serde_json::from_value(json!({"apiVersion":"networking.k8s.io/v1","kind":"NetworkPolicy","metadata":{"name":policy_name,"namespace":workspace.runtime.namespace(),"labels":labels},"spec":{"podSelector":{"matchLabels":{OPERATION_LABEL:operation.id.to_string()}},"policyTypes":["Ingress","Egress"],"ingress":[],"egress":[]}})).map_err(safe_error)?;
            policies
                .create(&PostParams::default(), &policy)
                .await
                .map_err(safe_error)?;
        }
        Ok(())
    }

    pub(crate) async fn remove_shrink_helpers(
        &self,
        workspace: &Workspace,
        operation: &StorageShrinkOperation,
    ) -> Result<bool, String> {
        let pods = Api::<Pod>::namespaced(self.client.clone(), workspace.runtime.namespace());
        let selector = format!("{OPERATION_LABEL}={}", operation.id);
        let list = pods
            .list(&ListParams::default().labels(&selector))
            .await
            .map_err(safe_error)?;
        if !list.items.is_empty() {
            for pod in list.items {
                let mut metadata = pod.metadata.clone();
                metadata.deletion_timestamp = None;
                self.shrink_owned(&metadata, workspace)?;
                let name = pod
                    .metadata
                    .name
                    .as_deref()
                    .ok_or("helper Pod name is missing")?;
                if operation
                    .helper_pod_uids
                    .get(name)
                    .is_some_and(|expected| pod.metadata.uid.as_ref() != Some(expected))
                {
                    return Err("helper Pod UID changed before cleanup".to_owned());
                }
                if pod.metadata.deletion_timestamp.is_some() {
                    continue;
                }
                let parameters = DeleteParams {
                    preconditions: Some(Preconditions {
                        uid: pod.metadata.uid.clone(),
                        resource_version: pod.metadata.resource_version.clone(),
                    }),
                    ..DeleteParams::default()
                };
                pods.delete(name, &parameters).await.map_err(safe_error)?;
            }
            return Ok(false);
        }
        let policies =
            Api::<NetworkPolicy>::namespaced(self.client.clone(), workspace.runtime.namespace());
        let name = format!("shrink-{}", operation.id.simple());
        if let Some(policy) = policies.get_opt(&name).await.map_err(safe_error)? {
            if policy.metadata.deletion_timestamp.is_some() {
                return Ok(false);
            }
            self.shrink_owned(&policy.metadata, workspace)?;
            if policy
                .metadata
                .labels
                .as_ref()
                .and_then(|labels| labels.get(OPERATION_LABEL))
                != Some(&operation.id.to_string())
            {
                return Err("helper NetworkPolicy ownership changed before cleanup".to_owned());
            }
            policies
                .delete(
                    &name,
                    &DeleteParams {
                        preconditions: Some(Preconditions {
                            uid: policy.metadata.uid,
                            resource_version: policy.metadata.resource_version,
                        }),
                        ..DeleteParams::default()
                    },
                )
                .await
                .map_err(safe_error)?;
            return Ok(false);
        }
        Ok(true)
    }
}

fn migration_pod(
    operation: &StorageShrinkOperation,
    metadata: ObjectMeta,
    stage: ShrinkPodStage,
    source: &WorkspaceHomeVolumeBinding,
    destination: Option<&WorkspaceHomeVolumeBinding>,
    image: &str,
) -> Result<Pod, String> {
    let mut volumes = vec![
        json!({"name":"source","persistentVolumeClaim":{"claimName":source.claim_name,"readOnly":true}}),
        json!({"name":"tmp","emptyDir":{"medium":"Memory","sizeLimit":"128Mi"}}),
    ];
    let mut mounts = vec![
        json!({"name":"source","mountPath":"/source","readOnly":true}),
        json!({"name":"tmp","mountPath":"/tmp"}),
    ];
    if matches!(stage, ShrinkPodStage::Preflight) {
        volumes.remove(0);
        mounts.remove(0);
    }
    if let Some(destination) = destination {
        volumes.push(json!({"name":"destination","persistentVolumeClaim":{"claimName":destination.claim_name,"readOnly":matches!(stage, ShrinkPodStage::Verify)}}));
        mounts.push(json!({"name":"destination","mountPath":"/destination","readOnly":matches!(stage, ShrinkPodStage::Verify)}));
    }
    serde_json::from_value(json!({"apiVersion":"v1","kind":"Pod","metadata":metadata,"spec":{"restartPolicy":"Never","activeDeadlineSeconds":86400,"automountServiceAccountToken":false,"enableServiceLinks":false,"securityContext":{"runAsUser":0,"runAsGroup":0,"seccompProfile":{"type":"RuntimeDefault"}},"containers":[{"name":"migrate","image":image,"command":["/bin/bash","-c"],"args":[stage.script()],"env":[{"name":"TARGET_BYTES","value":destination.map(|binding| (binding.capacity_gib * (1 << 30)).to_string()).unwrap_or_default()},{"name":"OPERATION_ID","value":operation.id.to_string()}],"securityContext":{"allowPrivilegeEscalation":false,"privileged":false,"readOnlyRootFilesystem":true,"capabilities":{"drop":["ALL"],"add":["CHOWN","DAC_OVERRIDE","FOWNER","FSETID"]}},"resources":{"requests":{"cpu":"100m","memory":"128Mi"},"limits":{"cpu":"1000m","memory":"512Mi"}},"terminationMessagePolicy":"File","volumeMounts":mounts}],"volumes":volumes}})).map_err(safe_error)
}

fn parse_usage_evidence(status: Option<&PodStatus>) -> Result<ShrinkUsage, String> {
    let message = status
        .and_then(|status| status.container_statuses.as_ref())
        .and_then(|containers| {
            containers
                .iter()
                .find(|container| container.name == "migrate")
        })
        .and_then(|container| container.state.as_ref())
        .and_then(|state| state.terminated.as_ref())
        .and_then(|terminated| terminated.message.as_ref())
        .ok_or("copy Pod did not publish its verification result")?;
    serde_json::from_str(message).map_err(|_| "copy Pod returned invalid usage evidence".to_owned())
}

fn binding_from_claim(
    claim: &PersistentVolumeClaim,
    workspace: &Workspace,
) -> Result<WorkspaceHomeVolumeBinding, String> {
    let bytes = claim
        .status
        .as_ref()
        .and_then(|status| status.capacity.as_ref())
        .and_then(|capacity| capacity.get("storage"))
        .and_then(|quantity| home_volume_capacity_bytes(&quantity.0))
        .ok_or("source claim capacity is unavailable")?;
    if bytes == 0 || bytes % (1 << 30) != 0 {
        return Err("source claim capacity must be an exact positive GiB quantity".to_owned());
    }
    Ok(WorkspaceHomeVolumeBinding {
        namespace: workspace.runtime.namespace().to_owned(),
        claim_name: claim
            .metadata
            .name
            .clone()
            .ok_or("source claim name is unavailable")?,
        claim_uid: claim
            .metadata
            .uid
            .clone()
            .ok_or("source claim UID is unavailable")?,
        capacity_gib: bytes / (1 << 30),
    })
}

fn validate_helper_spec(actual: &Pod, desired: &Pod) -> Result<(), String> {
    let actual_spec = actual.spec.as_ref().ok_or("helper Pod has no spec")?;
    let desired_spec = desired
        .spec
        .as_ref()
        .ok_or("helper Pod desired spec is missing")?;
    if actual_spec.containers.len() != 1
        || actual_spec
            .init_containers
            .as_ref()
            .is_some_and(|containers| !containers.is_empty())
        || actual_spec
            .ephemeral_containers
            .as_ref()
            .is_some_and(|containers| !containers.is_empty())
        || actual_spec.automount_service_account_token != Some(false)
        || actual_spec.volumes != desired_spec.volumes
        || actual_spec.security_context != desired_spec.security_context
        || actual_spec.host_network.unwrap_or(false)
        || actual_spec.host_pid.unwrap_or(false)
        || actual_spec.host_ipc.unwrap_or(false)
        || actual
            .metadata
            .owner_references
            .as_ref()
            .is_some_and(|owners| !owners.is_empty())
    {
        return Err("helper Pod security or volume manifest changed".to_owned());
    }
    let actual_container = &actual_spec.containers[0];
    let desired_container = &desired_spec.containers[0];
    if actual_container.name != desired_container.name
        || actual_container.image != desired_container.image
        || actual_container.command != desired_container.command
        || actual_container.args != desired_container.args
        || actual_container.env != desired_container.env
        || actual_container.env_from.is_some()
        || actual_container.volume_mounts != desired_container.volume_mounts
        || actual_container.security_context != desired_container.security_context
    {
        return Err("helper Pod command, image, credentials or mounts changed".to_owned());
    }
    Ok(())
}

fn validate_helper_policy(
    policy: &NetworkPolicy,
    operation: &StorageShrinkOperation,
) -> Result<(), String> {
    let spec = policy
        .spec
        .as_ref()
        .ok_or("helper NetworkPolicy spec is missing")?;
    let selector = spec
        .pod_selector
        .as_ref()
        .ok_or("helper NetworkPolicy selector is missing")?;
    let expected = BTreeMap::from([(OPERATION_LABEL.to_owned(), operation.id.to_string())]);
    let types: std::collections::BTreeSet<&str> = spec
        .policy_types
        .iter()
        .flatten()
        .map(String::as_str)
        .collect();
    if selector.match_labels.as_ref() != Some(&expected)
        || selector
            .match_expressions
            .as_ref()
            .is_some_and(|values| !values.is_empty())
        || spec.ingress.as_ref().is_some_and(|rules| !rules.is_empty())
        || spec.egress.as_ref().is_some_and(|rules| !rules.is_empty())
        || types != std::collections::BTreeSet::from(["Ingress", "Egress"])
    {
        return Err("helper NetworkPolicy rules/selectors changed; refusing reuse".to_owned());
    }
    Ok(())
}

fn safe_error(error: impl std::fmt::Display) -> String {
    format!("Kubernetes storage migration: {error}")
}

fn longhorn_resource() -> ApiResource {
    let mut resource =
        ApiResource::from_gvk(&GroupVersionKind::gvk("longhorn.io", "v1beta2", "Volume"));
    resource.plural = "volumes".to_owned();
    resource
}

fn validate_pv_identity(
    volume: &PersistentVolume,
    binding: &WorkspaceHomeVolumeBinding,
    identity: &StorageVolumeIdentity,
) -> Result<(), String> {
    let spec = volume.spec.as_ref().ok_or("PV spec is missing")?;
    let reference = spec.claim_ref.as_ref().ok_or("PV claimRef is missing")?;
    let csi = spec.csi.as_ref().ok_or("PV CSI identity is missing")?;
    if volume.metadata.name.as_ref() != Some(&identity.pv_name)
        || volume.metadata.uid.as_ref() != Some(&identity.pv_uid)
        || reference.namespace.as_ref() != Some(&binding.namespace)
        || reference.name.as_ref() != Some(&binding.claim_name)
        || reference.uid.as_ref() != Some(&binding.claim_uid)
        || csi.driver != identity.csi_driver
        || csi.volume_handle != identity.csi_volume_handle
        || spec.storage_class_name.as_ref() != Some(&identity.storage_class_name)
    {
        return Err("PV UID, claimRef UID, storage class or CSI backend identity changed; refusing reclamation".to_owned());
    }
    Ok(())
}

pub(crate) fn storage_shrink_image() -> Result<String, crate::storage::StorageError> {
    let image = std::env::var("MWC_STORAGE_SHRINK_IMAGE")
        .map_err(|_| crate::storage::StorageError::WorkspaceStorageShrinkUnavailable)?;
    let valid = image
        .rsplit_once("@sha256:")
        .is_some_and(|(repository, digest)| {
            !repository.is_empty()
                && !repository.contains('@')
                && !repository.chars().any(char::is_whitespace)
                && digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        });
    if !valid || std::env::var("MWC_STORAGE_SHRINK_XATTR_POLICY").as_deref() != Ok("posix-user") {
        return Err(crate::storage::StorageError::WorkspaceStorageShrinkUnavailable);
    }
    Ok(image)
}

#[cfg(test)]
#[path = "storage_shrink/tests.rs"]
mod tests;
