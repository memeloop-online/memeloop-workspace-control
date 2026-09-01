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

impl KubernetesCoordinator {
    pub async fn reconcile_port_mappings(
        &self,
        workspace: &Workspace,
        mappings: &[PortMapping],
    ) -> Result<(), ReconcileError> {
        let namespace = self
            .builder
            .installation_id
            .workspace_namespace(&workspace.short_id)?;
        let services = Api::<Service>::namespaced(self.client.clone(), &namespace);
        let ingresses = Api::<Ingress>::namespaced(self.client.clone(), &namespace);
        let policies = Api::<NetworkPolicy>::namespaced(self.client.clone(), &namespace);
        let apply = PatchParams::apply(FIELD_MANAGER);
        let mut desired = std::collections::BTreeSet::new();
        for mapping in mappings {
            let Some((service, auth_service, ingress, policy)) =
                self.builder.port_mapping_resources(workspace, mapping)?
            else {
                continue;
            };
            let name = resource_port_mappings::name(mapping);
            let auth_name = format!("{name}-auth");
            let policy_name = format!("{name}-ingress");
            verify_existing(&services, &name, &self.builder, workspace.id).await?;
            verify_existing(&services, &auth_name, &self.builder, workspace.id).await?;
            verify_existing(&ingresses, &name, &self.builder, workspace.id).await?;
            verify_existing(&policies, &policy_name, &self.builder, workspace.id).await?;
            services
                .patch(&name, &apply, &Patch::Apply(&service))
                .await?;
            services
                .patch(&auth_name, &apply, &Patch::Apply(&auth_service))
                .await?;
            ingresses
                .patch(&name, &apply, &Patch::Apply(&ingress))
                .await?;
            policies
                .patch(&policy_name, &apply, &Patch::Apply(&policy))
                .await?;
            desired.insert(name);
            desired.insert(auth_name);
        }

        let selector = format!(
            "{}={},{}={},{}",
            OWNER_INSTALLATION_LABEL,
            self.builder.installation_id,
            WORKSPACE_ID_LABEL,
            workspace.id,
            resource_port_mappings::PORT_MAPPING_ID_LABEL,
        );
        let list = ListParams::default().labels(&selector);
        for service in services.list(&list).await? {
            if let Some(name) = service.metadata.name.as_deref()
                && !desired.contains(name)
            {
                self.builder
                    .verify_delete_ownership(&service.metadata, workspace.id)?;
                services.delete(name, &DeleteParams::default()).await?;
            }
        }
        for ingress in ingresses.list(&list).await? {
            if let Some(name) = ingress.metadata.name.as_deref()
                && !desired.contains(name)
            {
                self.builder
                    .verify_delete_ownership(&ingress.metadata, workspace.id)?;
                ingresses.delete(name, &DeleteParams::default()).await?;
            }
        }
        for policy in policies.list(&list).await? {
            if let Some(name) = policy.metadata.name.as_deref()
                && !desired.contains(name.trim_end_matches("-ingress"))
            {
                self.builder
                    .verify_delete_ownership(&policy.metadata, workspace.id)?;
                policies.delete(name, &DeleteParams::default()).await?;
            }
        }
        Ok(())
    }
}
