use memeloop_workspace_control::{
    auth::Role,
    crypto::EnvelopeCipher,
    injections::{InjectionItem, InjectionKind, InjectionScope, InjectionValue},
    quota::Resources,
    storage::{
        CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate, Database,
        IdempotencyDecision, InjectionScopeRef, StorageError,
    },
    workspaces::{AccessMode, WorkspaceAction, WorkspaceRuntimeProfile, WorkspaceState},
};
use std::collections::BTreeMap;

const ADMIN_TOKEN: &str = "admin-token-0000000000000000000000000000";

async fn database() -> Database {
    let database = Database::connect("sqlite::memory:", "business-test".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
        .upsert_image_policy("registry.example/workspace:1", true, 1)
        .await
        .unwrap();
    database
}

#[tokio::test]
async fn unconfigured_image_allowlist_defaults_to_deny() {
    let database = Database::connect("sqlite::memory:", "default-deny".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let admin = database
        .create_user("Admin", ADMIN_TOKEN, true, 100)
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Default deny".to_owned(),
                owner_user_id: admin.user_id,
            },
            101,
        )
        .await
        .unwrap();
    let result = database
        .create_workspace(
            CreateWorkspace {
                organization_id: organization.id,
                owner_id: admin.user_id,
                name: "must-be-rejected".to_owned(),
                template_id: None,
                organization_injection_refs: None,
                user_injection_refs: None,
                image: "registry.example/workspace:unconfigured".to_owned(),
                access_mode: AccessMode::Internal,
                resources: Resources {
                    cpu_millis: 1_000,
                    memory_mib: 2_048,
                    gpu_count: 0,
                    disk_gib: 20,
                },
            },
            admin.user_id,
            102,
        )
        .await;
    assert!(matches!(result, Err(StorageError::ImageNotAllowed)));
}

#[tokio::test]
async fn inline_injection_failure_rolls_back_workspace_and_first_job() {
    let database = database().await;
    let admin = database
        .create_user("Admin", ADMIN_TOKEN, true, 100)
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Atomic inline".to_owned(),
                owner_user_id: admin.user_id,
            },
            101,
        )
        .await
        .unwrap();
    let cipher =
        EnvelopeCipher::from_base64(&EnvelopeCipher::generate_base64_key().unwrap()).unwrap();
    let result = database
        .create_workspace_with_inline_injections(
            CreateWorkspace {
                organization_id: organization.id,
                owner_id: admin.user_id,
                name: "must-roll-back".to_owned(),
                template_id: None,
                organization_injection_refs: None,
                user_injection_refs: None,
                image: "registry.example/workspace:1".to_owned(),
                access_mode: AccessMode::Internal,
                resources: Resources {
                    cpu_millis: 1_000,
                    memory_mib: 2_048,
                    gpu_count: 0,
                    disk_gib: 20,
                },
            },
            &cipher,
            &[InjectionItem {
                key: "invalid-binary".to_owned(),
                kind: InjectionKind::SecretFile,
                target: "/run/secrets/invalid".to_owned(),
                value: InjectionValue::Base64("not base64!".to_owned()),
                sensitive: true,
                locked: false,
                version: 0,
                file_mode: Some(0o600),
                owner: None,
                group: None,
                template_selector: None,
                labels: BTreeMap::new(),
            }],
            admin.user_id,
            102,
        )
        .await;
    assert!(matches!(
        result,
        Err(StorageError::InvalidEncryptedInjection)
    ));
    assert!(
        database
            .list_workspaces(organization.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(database.job_counts().await.unwrap().pending, 0);
}

#[tokio::test]
async fn image_allowlist_and_template_contract_are_admitted_atomically() {
    let database = database().await;
    let admin = database
        .create_user("Admin", ADMIN_TOKEN, true, 100)
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Catalog Org".to_owned(),
                owner_user_id: admin.user_id,
            },
            101,
        )
        .await
        .unwrap();
    let image = "registry.example/workspace@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    database
        .upsert_image_policy(image, true, 102)
        .await
        .unwrap();
    let resources = Resources {
        cpu_millis: 2_000,
        memory_mib: 4_096,
        gpu_count: 0,
        disk_gib: 50,
    };
    let template = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(organization.id),
                name: "Standard".to_owned(),
                runtime_profile: WorkspaceRuntimeProfile::RustDev,
                image: image.to_owned(),
                access_mode: AccessMode::Internal,
                resources,
            },
            103,
        )
        .await
        .unwrap();
    let command = CreateWorkspace {
        organization_id: organization.id,
        owner_id: admin.user_id,
        name: "contract-ok".to_owned(),
        template_id: Some(template.id),
        organization_injection_refs: Some(vec!["org-key".to_owned()]),
        user_injection_refs: Some(Vec::new()),
        image: image.to_owned(),
        access_mode: AccessMode::Internal,
        resources,
    };
    let created = database
        .create_workspace(command.clone(), admin.user_id, 104)
        .await
        .unwrap();
    assert_eq!(created.runtime_profile, WorkspaceRuntimeProfile::RustDev);
    assert_eq!(
        database
            .get_workspace(created.id)
            .await
            .unwrap()
            .runtime_profile,
        WorkspaceRuntimeProfile::RustDev
    );
    let refs = database.workspace_injection_refs(created.id).await.unwrap();
    assert_eq!(refs.organization, Some(vec!["org-key".to_owned()]));
    assert_eq!(refs.user, Some(Vec::new()));
    let metrics = database.workspace_metrics().await.unwrap();
    assert_eq!(metrics.states.get("provisioning"), Some(&1));
    assert_eq!(metrics.users.len(), 1);
    assert_eq!(metrics.users[0].user_id, admin.user_id);
    assert_eq!(metrics.users[0].resources, resources);
    let mut changed = command;
    changed.name = "contract-changed".to_owned();
    changed.resources.cpu_millis = 3_000;
    assert!(matches!(
        database.create_workspace(changed, admin.user_id, 105).await,
        Err(StorageError::TemplateNotFound)
    ));
    let not_allowed = CreateWorkspace {
        organization_id: organization.id,
        owner_id: admin.user_id,
        name: "image-rejected".to_owned(),
        template_id: None,
        organization_injection_refs: None,
        user_injection_refs: None,
        image: "registry.example/unapproved:latest".to_owned(),
        access_mode: AccessMode::Internal,
        resources,
    };
    assert!(matches!(
        database
            .create_workspace(not_allowed, admin.user_id, 106)
            .await,
        Err(StorageError::ImageNotAllowed)
    ));
}

#[tokio::test]
async fn authentication_and_organization_membership_are_persisted() {
    let database = database().await;
    let admin = database
        .create_user("Admin", ADMIN_TOKEN, true, 100)
        .await
        .unwrap();
    assert!(database.authenticate("wrong").await.unwrap().is_none());
    let authenticated = database.authenticate(ADMIN_TOKEN).await.unwrap().unwrap();
    assert_eq!(authenticated.user_id, admin.user_id);
    assert!(authenticated.system_admin);

    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Example".to_owned(),
                owner_user_id: admin.user_id,
            },
            101,
        )
        .await
        .unwrap();
    let authenticated = database.authenticate(ADMIN_TOKEN).await.unwrap().unwrap();
    assert_eq!(authenticated.memberships.len(), 1);
    assert_eq!(
        authenticated.memberships[0].organization_id,
        organization.id
    );
}

#[tokio::test]
async fn workspace_creation_enforces_quota_and_enqueues_lifecycle_actions() {
    let database = database().await;
    let admin = database
        .create_user("Admin", ADMIN_TOKEN, true, 100)
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Quota Org".to_owned(),
                owner_user_id: admin.user_id,
            },
            101,
        )
        .await
        .unwrap();
    database
        .set_organization_quota(
            organization.id,
            Resources {
                cpu_millis: 1_000,
                memory_mib: 2_048,
                gpu_count: 0,
                disk_gib: 20,
            },
            102,
        )
        .await
        .unwrap();
    database
        .set_user_quota(
            admin.user_id,
            Resources {
                cpu_millis: 700,
                memory_mib: 2_048,
                gpu_count: 0,
                disk_gib: 20,
            },
            102,
        )
        .await
        .unwrap();

    let command = |name: &str, cpu_millis| CreateWorkspace {
        organization_id: organization.id,
        owner_id: admin.user_id,
        name: name.to_owned(),
        template_id: None,
        organization_injection_refs: None,
        user_injection_refs: None,
        image: "registry.example/workspace:1".to_owned(),
        access_mode: AccessMode::Internal,
        resources: Resources {
            cpu_millis,
            memory_mib: 512,
            gpu_count: 0,
            disk_gib: 5,
        },
    };
    let workspace = database
        .create_workspace(command("first", 600), admin.user_id, 103)
        .await
        .unwrap();
    assert_eq!(workspace.state, WorkspaceState::Provisioning);
    assert_eq!(workspace.runtime_profile, WorkspaceRuntimeProfile::Standard);
    let created_events = database
        .list_events(organization.id, None, 100)
        .await
        .unwrap();
    assert_eq!(created_events.len(), 1);
    assert_eq!(created_events[0].kind, "workspace.state_changed");
    assert_eq!(created_events[0].workspace_id, Some(workspace.id));
    assert_eq!(created_events[0].payload["state"], "provisioning");
    assert!(matches!(
        database
            .create_workspace(command("second", 200), admin.user_id, 104)
            .await,
        Err(StorageError::Quota(_))
    ));
    assert_eq!(
        database
            .list_workspaces(organization.id)
            .await
            .unwrap()
            .len(),
        1
    );

    let ready = database
        .transition_workspace(workspace.id, WorkspaceAction::MarkReady, admin.user_id, 105)
        .await
        .unwrap();
    assert_eq!(ready.state, WorkspaceState::Ready);
    let stopping = database
        .transition_workspace(workspace.id, WorkspaceAction::Stop, admin.user_id, 106)
        .await
        .unwrap();
    assert_eq!(stopping.state, WorkspaceState::Stopping);
    assert_eq!(ready.generation, 1);
    assert_eq!(stopping.generation, 2);
    let events = database
        .list_events(organization.id, None, 100)
        .await
        .unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[2].payload["action"], "stop");
    assert_eq!(events[2].payload["state"], "stopping");
}

#[tokio::test]
async fn idempotency_key_conflicts_and_replays_exact_response() {
    let database = database().await;
    assert_eq!(
        database
            .begin_idempotency("user:create-workspace", "request-1", "hash-a", 10, 100)
            .await
            .unwrap(),
        IdempotencyDecision::Reserved
    );
    assert_eq!(
        database
            .begin_idempotency("user:create-workspace", "request-1", "hash-a", 11, 100)
            .await
            .unwrap(),
        IdempotencyDecision::InProgress
    );
    assert_eq!(
        database
            .begin_idempotency("user:create-workspace", "request-1", "hash-b", 11, 100)
            .await
            .unwrap(),
        IdempotencyDecision::Conflict
    );
    database
        .finish_idempotency(
            "user:create-workspace",
            "request-1",
            "hash-a",
            201,
            r#"{"id":"one"}"#,
        )
        .await
        .unwrap();
    let IdempotencyDecision::Replay(replay) = database
        .begin_idempotency("user:create-workspace", "request-1", "hash-a", 12, 100)
        .await
        .unwrap()
    else {
        panic!("expected replay");
    };
    assert_eq!(replay.status_code, 201);
    assert_eq!(replay.response_json, r#"{"id":"one"}"#);

    assert_eq!(
        database
            .begin_idempotency("user:create-workspace", "request-1", "hash-c", 101, 200)
            .await
            .unwrap(),
        IdempotencyDecision::Reserved
    );
}

#[tokio::test]
async fn confirmed_deletion_scrubs_sensitive_workspace_state_and_keeps_a_tombstone() {
    let database = database().await;
    let cipher =
        EnvelopeCipher::from_base64("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap();
    let admin = database
        .create_user("Admin", ADMIN_TOKEN, true, 100)
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Deletion Org".to_owned(),
                owner_user_id: admin.user_id,
            },
            101,
        )
        .await
        .unwrap();
    let workspace = database
        .create_workspace(
            CreateWorkspace {
                organization_id: organization.id,
                owner_id: admin.user_id,
                name: "sensitive-name".to_owned(),
                template_id: None,
                organization_injection_refs: None,
                user_injection_refs: None,
                image: "registry.example/workspace:1".to_owned(),
                access_mode: AccessMode::Internal,
                resources: Resources {
                    cpu_millis: 1_000,
                    memory_mib: 2_048,
                    gpu_count: 0,
                    disk_gib: 20,
                },
            },
            admin.user_id,
            102,
        )
        .await
        .unwrap();
    let scope = InjectionScopeRef {
        scope: InjectionScope::Workspace,
        scope_id: workspace.id,
    };
    database
        .replace_injection(
            &cipher,
            scope,
            InjectionItem {
                key: "secret".to_owned(),
                kind: InjectionKind::SecretFile,
                target: "/home/workspace/secret".to_owned(),
                value: InjectionValue::Utf8("delete-me".to_owned()),
                sensitive: true,
                locked: false,
                version: 0,
                file_mode: Some(0o600),
                owner: Some("workspace".to_owned()),
                group: Some("workspace".to_owned()),
                template_selector: None,
                labels: BTreeMap::new(),
            },
            admin.user_id,
            103,
        )
        .await
        .unwrap();
    database
        .ensure_workspace_ssh_identity(&cipher, workspace.id, 104)
        .await
        .unwrap();
    database
        .transition_workspace(workspace.id, WorkspaceAction::MarkReady, admin.user_id, 105)
        .await
        .unwrap();
    let ticket = database
        .issue_web_shell_ticket(organization.id, workspace.id, admin.user_id, 106, 60)
        .await
        .unwrap();
    database
        .transition_workspace(workspace.id, WorkspaceAction::Delete, admin.user_id, 107)
        .await
        .unwrap();
    database
        .transition_workspace(
            workspace.id,
            WorkspaceAction::MarkDeleted,
            admin.user_id,
            108,
        )
        .await
        .unwrap();
    assert!(matches!(
        database.get_workspace(workspace.id).await,
        Err(StorageError::WorkspaceNotFound)
    ));
    assert!(
        database
            .list_injection_summaries(scope)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        database
            .workspace_ssh_public_identity(workspace.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        database
            .consume_web_shell_ticket(&ticket.ticket, workspace.id, 109)
            .await
            .unwrap()
            .is_none()
    );
    let snapshot = serde_json::to_string(&database.export_snapshot(110).await.unwrap()).unwrap();
    assert!(snapshot.contains("workspace_tombstones"));
    assert!(!snapshot.contains("delete-me"));
    assert!(!snapshot.contains("sensitive-name"));
    assert!(!snapshot.contains("sensitive-image"));
}

#[tokio::test]
async fn user_injection_change_reconciles_every_workspace_the_user_can_access() {
    let database = database().await;
    let admin = database
        .create_user("Admin", ADMIN_TOKEN, true, 100)
        .await
        .unwrap();
    let member = database
        .create_user(
            "Member",
            "member-token-000000000000000000000000000",
            false,
            101,
        )
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Reconcile Org".to_owned(),
                owner_user_id: admin.user_id,
            },
            102,
        )
        .await
        .unwrap();
    database
        .upsert_membership(organization.id, member.user_id, Role::Member, 103)
        .await
        .unwrap();
    database
        .create_workspace(
            CreateWorkspace {
                organization_id: organization.id,
                owner_id: admin.user_id,
                name: "shared".to_owned(),
                template_id: None,
                organization_injection_refs: None,
                user_injection_refs: None,
                image: "registry.example/workspace:1".to_owned(),
                access_mode: AccessMode::Public,
                resources: Resources {
                    cpu_millis: 500,
                    memory_mib: 512,
                    gpu_count: 0,
                    disk_gib: 5,
                },
            },
            admin.user_id,
            104,
        )
        .await
        .unwrap();

    let affected = database
        .enqueue_injection_reconciles(
            InjectionScopeRef {
                scope: InjectionScope::User,
                scope_id: member.user_id,
            },
            105,
        )
        .await
        .unwrap();
    assert_eq!(affected, 1);
    assert_eq!(database.job_counts().await.unwrap().pending, 2);
}
