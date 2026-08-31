use std::time::Duration;

use memeloop_workspace_control::{
    crypto::EnvelopeCipher,
    quota::Resources,
    storage::{
        CreateOrganization, CreateWebhookSubscription, CreateWorkspace, CreateWorkspaceTemplate,
        Database,
    },
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::{AccessMode, WorkspaceObservation},
};
use uuid::Uuid;

const TOKEN: &str = "webhook-admin-00000000000000000000000000000";

#[tokio::test]
async fn webhook_secret_is_encrypted_and_workspace_events_enqueue_durable_delivery() {
    let database = Database::connect("sqlite::memory:", "webhook-test".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
        .upsert_image_policy("registry.example/workspace:1", true, 99)
        .await
        .unwrap();
    let cipher =
        EnvelopeCipher::from_base64("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap();
    let admin = database.create_user("Admin", TOKEN, true, 1).await.unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Webhook Org".to_owned(),
                owner_user_id: admin.user_id,
            },
            2,
        )
        .await
        .unwrap();
    let signing_secret = "never-store-this-webhook-secret-in-plaintext";
    let subscription = database
        .create_webhook_subscription(
            &cipher,
            CreateWebhookSubscription {
                organization_id: organization.id,
                url: "https://hooks.example.com/memeloop".to_owned(),
                event_prefix: "workspace.".to_owned(),
                signing_secret: signing_secret.to_owned(),
            },
            admin.user_id,
            3,
        )
        .await
        .unwrap();
    assert!(
        !serde_json::to_string(&subscription)
            .unwrap()
            .contains(signing_secret)
    );

    let template = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(organization.id),
                yaml: WorkspaceTemplateDocument::new(
                    "Webhook workspace",
                    WorkspaceTemplateSpec::standard(
                        "registry.example/workspace:1",
                        AccessMode::Internal,
                        Resources {
                            cpu_millis: 1_000,
                            memory_mib: 2_048,
                            gpu_count: 0,
                            disk_gib: 20,
                        },
                    ),
                )
                .to_yaml()
                .unwrap(),
            },
            true,
            3,
        )
        .await
        .unwrap();

    let workspace = database
        .create_workspace(
            CreateWorkspace {
                organization_id: organization.id,
                owner_id: admin.user_id,
                name: "delivery-source".to_owned(),
                template_id: template.id,
                resources: None,
                organization_injection_refs: None,
                user_injection_refs: None,
            },
            true,
            admin.user_id,
            4,
        )
        .await
        .unwrap();
    database
        .record_workspace_observation(workspace.id, WorkspaceObservation::Ready, admin.user_id, 5)
        .await
        .unwrap();
    assert!(database.job_counts().await.unwrap().pending >= 2);

    let mut delivery_job = None;
    for index in 0..8 {
        let Some(job) = database
            .claim_job("webhook-test-worker", 10 + index, Duration::from_secs(60))
            .await
            .unwrap()
        else {
            break;
        };
        if job.kind == "deliver_webhook" {
            delivery_job = Some(job);
            break;
        }
        database
            .complete_job(job.id, "webhook-test-worker", 10 + index)
            .await
            .unwrap();
    }
    let job = delivery_job.expect("a matching webhook delivery job");
    let event_id = Uuid::parse_str(job.payload["event_id"].as_str().unwrap()).unwrap();
    let loaded = database
        .load_webhook_delivery(&cipher, subscription.id, event_id)
        .await
        .unwrap();
    assert_eq!(loaded.event.organization_id, organization.id);
    assert_eq!(loaded.signing_secret.as_slice(), signing_secret.as_bytes());

    let snapshot = database.export_snapshot(20).await.unwrap();
    assert!(
        !serde_json::to_string(&snapshot)
            .unwrap()
            .contains(signing_secret)
    );
}
