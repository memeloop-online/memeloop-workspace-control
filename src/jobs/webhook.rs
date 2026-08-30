use std::time::Duration;

use hmac::{Hmac, Mac};
use reqwest::redirect::Policy;
use serde::Deserialize;
use sha2::Sha256;
use uuid::Uuid;

use crate::{
    crypto::EnvelopeCipher,
    observability::{Observability, UpstreamKind},
    storage::{ClaimedJob, Database},
};

use super::{JobHandler, JobHandlerError, WorkspaceReconcileHandler};

pub struct WebhookDeliveryHandler {
    database: Database,
    cipher: EnvelopeCipher,
    client: reqwest::Client,
    observability: Observability,
}

impl WebhookDeliveryHandler {
    pub fn new(
        database: Database,
        cipher: EnvelopeCipher,
        observability: Observability,
    ) -> Result<Self, JobHandlerError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(Policy::none())
            .build()
            .map_err(job_error)?;
        Ok(Self {
            database,
            cipher,
            client,
            observability,
        })
    }

    async fn deliver(&self, job: &ClaimedJob) -> Result<(), JobHandlerError> {
        let payload: DeliveryJob =
            serde_json::from_value(job.payload.clone()).map_err(job_error)?;
        let delivery = self
            .database
            .load_webhook_delivery(&self.cipher, payload.subscription_id, payload.event_id)
            .await
            .map_err(job_error)?;
        let body = serde_json::to_vec(&delivery.event).map_err(job_error)?;
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&delivery.signing_secret).map_err(job_error)?;
        mac.update(&body);
        let signature = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let request = self.observability.begin_upstream(UpstreamKind::Webhook);
        let response = self
            .client
            .post(&delivery.subscription.url)
            .header("content-type", "application/json")
            .header("x-memeloop-event-id", delivery.event.id.to_string())
            .header("x-memeloop-signature-256", format!("sha256={signature}"))
            .body(body)
            .send()
            .await
            .map_err(job_error)?;
        if !response.status().is_success() {
            return Err(JobHandlerError(format!(
                "webhook endpoint returned HTTP {}",
                response.status()
            )));
        }
        request.success();
        Ok(())
    }
}

pub struct ControlPlaneJobHandler {
    workspace: Option<WorkspaceReconcileHandler>,
    webhook: WebhookDeliveryHandler,
}

impl ControlPlaneJobHandler {
    pub fn new(
        workspace: Option<WorkspaceReconcileHandler>,
        webhook: WebhookDeliveryHandler,
    ) -> Self {
        Self { workspace, webhook }
    }
}

impl JobHandler for ControlPlaneJobHandler {
    async fn handle(&self, job: &ClaimedJob) -> Result<(), JobHandlerError> {
        match job.kind.as_str() {
            "reconcile_workspace" => {
                self.workspace
                    .as_ref()
                    .ok_or_else(|| {
                        JobHandlerError("Kubernetes coordination is disabled".to_owned())
                    })?
                    .handle(job)
                    .await
            }
            "deliver_webhook" => self.webhook.deliver(job).await,
            _ => Err(JobHandlerError(format!(
                "unsupported background job kind {}",
                job.kind
            ))),
        }
    }
}

#[derive(Debug, Deserialize)]
struct DeliveryJob {
    subscription_id: Uuid,
    event_id: Uuid,
}
fn job_error(error: impl std::fmt::Display) -> JobHandlerError {
    JobHandlerError(error.to_string())
}
