use k8s_openapi::api::core::v1::Secret;
use kube::{
    Api,
    api::{DeleteParams, Patch, PatchParams, Preconditions},
    core::DynamicObject,
};

use crate::workspaces::Workspace;

use super::super::super::DesiredResources;
use super::super::{FIELD_MANAGER, KubernetesCoordinator, ReconcileError, verify_existing};

impl KubernetesCoordinator {
    pub(super) async fn apply_web_shell_envoy_filter(
        &self,
        workspace: &Workspace,
        desired: &DesiredResources,
    ) -> Result<(), ReconcileError> {
        let names = self.builder.runtime_names(workspace)?;
        let api = Api::<DynamicObject>::namespaced_with(
            self.client.clone(),
            &self.builder.higress_namespace,
            &super::super::super::envoy_filter::api_resource(),
        );
        if let Some(filter) = &desired.web_shell_envoy_filter {
            self.verify_ttyd_mtls_secrets().await?;
            verify_existing(
                &api,
                &names.resources.web_shell_envoy_filter,
                &self.builder,
                workspace.id,
            )
            .await?;
            api.patch(
                &names.resources.web_shell_envoy_filter,
                &PatchParams::apply(FIELD_MANAGER),
                &Patch::Apply(filter),
            )
            .await?;
        } else if let Some(existing) = api
            .get_metadata_opt(&names.resources.web_shell_envoy_filter)
            .await?
        {
            self.builder
                .verify_delete_ownership(&existing.metadata, workspace.id)?;
            let uid = existing
                .metadata
                .uid
                .ok_or(ReconcileError::MissingEnvoyFilterUid)?;
            api.delete(
                &names.resources.web_shell_envoy_filter,
                &DeleteParams::default().preconditions(Preconditions {
                    uid: Some(uid),
                    resource_version: None,
                }),
            )
            .await?;
        }
        Ok(())
    }

    async fn verify_ttyd_mtls_secrets(&self) -> Result<(), ReconcileError> {
        let mtls = self
            .builder
            .ttyd_mtls
            .as_ref()
            .ok_or(ReconcileError::MissingTtydMtlsConfig)?;
        let secrets =
            Api::<Secret>::namespaced(self.client.clone(), &self.builder.higress_namespace);
        let client = secrets
            .get_opt(&mtls.higress_client_secret_name)
            .await?
            .ok_or(ReconcileError::MissingTtydMtlsSecret)?;
        let companion = secrets
            .get_opt(&format!("{}-cacert", mtls.higress_client_secret_name))
            .await?
            .ok_or(ReconcileError::MissingTtydMtlsCompanion)?;
        let has = |secret: &Secret, key: &str| {
            secret
                .data
                .as_ref()
                .and_then(|data| data.get(key))
                .is_some_and(|value| !value.0.is_empty())
        };
        if !has(&client, "tls.crt") || !has(&client, "tls.key") {
            return Err(ReconcileError::InvalidTtydMtlsClientSecret);
        }
        if !has(&companion, "cacert") {
            return Err(ReconcileError::InvalidTtydMtlsCompanion);
        }
        Ok(())
    }
}
