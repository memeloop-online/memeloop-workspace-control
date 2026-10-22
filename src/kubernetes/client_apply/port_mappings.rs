use std::collections::BTreeSet;

use k8s_openapi::api::{
    core::v1::Service,
    networking::v1::{Ingress, NetworkPolicy},
};
use kube::{
    Api,
    api::{DeleteParams, ListParams, Patch, PatchParams},
};

use crate::{storage::PortMapping, workspaces::Workspace};

use super::super::super::{
    OWNER_INSTALLATION_LABEL, WORKSPACE_ID_LABEL, port_mappings as resource_port_mappings,
};
use super::super::{FIELD_MANAGER, KubernetesCoordinator, ReconcileError, verify_existing};
use super::{observe_runtime_stage, request_observation::observe_request};

#[cfg(test)]
#[path = "port_mappings_tests.rs"]
mod tests;

struct PortMappingInventory {
    services: Vec<Service>,
    ingresses: Vec<Ingress>,
    policies: Vec<NetworkPolicy>,
}

impl KubernetesCoordinator {
    pub async fn reconcile_port_mappings(
        &self,
        workspace: &Workspace,
        mappings: &[PortMapping],
    ) -> Result<(), ReconcileError> {
        observe_runtime_stage(
            workspace,
            "port_mappings",
            self.reconcile_port_mappings_inner(workspace, mappings),
        )
        .await
    }

    async fn reconcile_port_mappings_inner(
        &self,
        workspace: &Workspace,
        mappings: &[PortMapping],
    ) -> Result<(), ReconcileError> {
        self.apply_http_proxy_routes(workspace, mappings).await?;
        let (desired, desired_filters) = self
            .apply_desired_port_mappings(workspace, mappings)
            .await?;
        let inventory = self.list_port_mapping_inventory(workspace).await?;
        self.prune_port_mapping_inventory(workspace, &desired, inventory)
            .await?;
        if self.builder.http_proxy_enabled() {
            self.prune_mapping_tls_filters(workspace, &desired_filters)
                .await?;
        }
        Ok(())
    }

    async fn apply_desired_port_mappings(
        &self,
        workspace: &Workspace,
        mappings: &[PortMapping],
    ) -> Result<(BTreeSet<String>, BTreeSet<String>), ReconcileError> {
        let namespace = workspace.runtime.namespace();
        let services = Api::<Service>::namespaced(self.client.clone(), namespace);
        let ingresses = Api::<Ingress>::namespaced(self.client.clone(), namespace);
        let policies = Api::<NetworkPolicy>::namespaced(self.client.clone(), namespace);
        let apply = PatchParams::apply(FIELD_MANAGER);
        let mut desired = BTreeSet::new();
        let mut desired_filters = BTreeSet::new();
        for mapping in mappings {
            let Some((service, ingress, policy)) =
                self.builder.port_mapping_resources(workspace, mapping)?
            else {
                continue;
            };
            let name = resource_port_mappings::name(mapping);
            let policy_name = format!("{name}-ingress");
            if let Some(filter) = self.apply_mapping_tls_filter(workspace, mapping).await? {
                desired_filters.insert(filter);
            }
            verify_existing(&services, &name, &self.builder, workspace.id).await?;
            verify_existing(&ingresses, &name, &self.builder, workspace.id).await?;
            verify_existing(&policies, &policy_name, &self.builder, workspace.id).await?;
            observe_request(
                workspace.id,
                "Service",
                &name,
                "apply",
                services.patch(&name, &apply, &Patch::Apply(&service)),
            )
            .await?;
            observe_request(
                workspace.id,
                "Ingress",
                &name,
                "apply",
                ingresses.patch(&name, &apply, &Patch::Apply(&ingress)),
            )
            .await?;
            observe_request(
                workspace.id,
                "NetworkPolicy",
                &policy_name,
                "apply",
                policies.patch(&policy_name, &apply, &Patch::Apply(&policy)),
            )
            .await?;
            desired.insert(name);
        }
        Ok((desired, desired_filters))
    }

    async fn list_port_mapping_inventory(
        &self,
        workspace: &Workspace,
    ) -> Result<PortMappingInventory, ReconcileError> {
        let namespace = workspace.runtime.namespace();
        let services = Api::<Service>::namespaced(self.client.clone(), namespace);
        let ingresses = Api::<Ingress>::namespaced(self.client.clone(), namespace);
        let policies = Api::<NetworkPolicy>::namespaced(self.client.clone(), namespace);
        let selector = format!(
            "{}={},{}={},{}",
            OWNER_INSTALLATION_LABEL,
            self.builder.installation_id,
            WORKSPACE_ID_LABEL,
            workspace.id,
            resource_port_mappings::PORT_MAPPING_ID_LABEL,
        );
        let list = ListParams::default().labels(&selector);
        // Fetch independent cleanup inventories together. Apply the same ownership
        // checks and deletions in their original order only after all reads succeed.
        let (services, ingresses, policies) = tokio::try_join!(
            observe_request(
                workspace.id,
                "Service",
                namespace,
                "list",
                services.list(&list)
            ),
            observe_request(
                workspace.id,
                "Ingress",
                namespace,
                "list",
                ingresses.list(&list)
            ),
            observe_request(
                workspace.id,
                "NetworkPolicy",
                namespace,
                "list",
                policies.list(&list)
            ),
        )?;
        Ok(PortMappingInventory {
            services: services.items,
            ingresses: ingresses.items,
            policies: policies.items,
        })
    }

    async fn prune_port_mapping_inventory(
        &self,
        workspace: &Workspace,
        desired: &BTreeSet<String>,
        inventory: PortMappingInventory,
    ) -> Result<(), ReconcileError> {
        let namespace = workspace.runtime.namespace();
        let services = Api::<Service>::namespaced(self.client.clone(), namespace);
        let ingresses = Api::<Ingress>::namespaced(self.client.clone(), namespace);
        let policies = Api::<NetworkPolicy>::namespaced(self.client.clone(), namespace);
        for service in inventory.services {
            if let Some(name) = service.metadata.name.as_deref()
                && !desired.contains(name)
            {
                self.builder
                    .verify_delete_ownership(&service.metadata, workspace.id)?;
                observe_request(
                    workspace.id,
                    "Service",
                    name,
                    "delete",
                    services.delete(name, &DeleteParams::default()),
                )
                .await?;
            }
        }
        for ingress in inventory.ingresses {
            if let Some(name) = ingress.metadata.name.as_deref()
                && !desired.contains(name)
            {
                self.builder
                    .verify_delete_ownership(&ingress.metadata, workspace.id)?;
                observe_request(
                    workspace.id,
                    "Ingress",
                    name,
                    "delete",
                    ingresses.delete(name, &DeleteParams::default()),
                )
                .await?;
            }
        }
        for policy in inventory.policies {
            if let Some(name) = policy.metadata.name.as_deref()
                && !desired.contains(name.trim_end_matches("-ingress"))
            {
                self.builder
                    .verify_delete_ownership(&policy.metadata, workspace.id)?;
                observe_request(
                    workspace.id,
                    "NetworkPolicy",
                    name,
                    "delete",
                    policies.delete(name, &DeleteParams::default()),
                )
                .await?;
            }
        }
        Ok(())
    }
}
