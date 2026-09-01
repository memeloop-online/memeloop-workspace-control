use memeloop_workspace_control::{
    quota::Resources,
    storage::{
        CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate, Database, StorageError,
        hash_secret,
    },
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::AccessMode,
};
use uuid::Uuid;

const USER_TOKEN: &str = "port-mapping-user-token-000000000000000000000";

async fn seeded_database() -> (Database, Uuid, Uuid, Uuid) {
    let database = Database::connect("sqlite::memory:", "port-mapping-test".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
        .upsert_image_policy("registry.example/workspace:1", true, 99)
        .await
        .unwrap();

    let user = database
        .create_user("Port Mapping User", USER_TOKEN, true, 100)
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Port Mapping Org".to_owned(),
                owner_user_id: user.user_id,
            },
            101,
        )
        .await
        .unwrap();
    let template = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(organization.id),
                yaml: WorkspaceTemplateDocument::new(
                    "Port Mapping Template",
                    WorkspaceTemplateSpec::standard(
                        "registry.example/workspace:1",
                        AccessMode::Internal,
                        Resources {
                            cpu_millis: 500,
                            memory_mib: 512,
                            gpu_count: 0,
                            disk_gib: 5,
                        },
                    ),
                )
                .to_yaml()
                .unwrap(),
            },
            true,
            102,
        )
        .await
        .unwrap();
    let workspace = database
        .create_workspace(
            CreateWorkspace {
                organization_id: organization.id,
                owner_id: user.user_id,
                name: "port-mapping-workspace".to_owned(),
                template_id: template.id,
                resources: None,
                organization_injection_refs: None,
                user_injection_refs: None,
            },
            true,
            user.user_id,
            103,
        )
        .await
        .unwrap();

    (database, organization.id, workspace.id, user.user_id)
}

#[tokio::test]
async fn create_and_list_port_mappings_converge_duplicate_ports() {
    let (database, organization_id, workspace_id, user_id) = seeded_database().await;

    let first = database.create_port_mapping(
        organization_id,
        workspace_id,
        3000,
        Some("  frontend  "),
        user_id,
        110,
    );
    let duplicate = database.create_port_mapping(
        organization_id,
        workspace_id,
        3000,
        Some("other name is ignored"),
        user_id,
        111,
    );
    let (first, duplicate) = tokio::join!(first, duplicate);
    let first = first.unwrap();
    let duplicate = duplicate.unwrap();
    assert_eq!(first.id, duplicate.id);
    assert_eq!(first.display_name.as_deref(), Some("frontend"));

    let second = database
        .create_port_mapping(
            organization_id,
            workspace_id,
            4000,
            Some("backend"),
            user_id,
            112,
        )
        .await
        .unwrap();
    let mappings = database.list_port_mappings(workspace_id).await.unwrap();
    assert_eq!(mappings.len(), 2);
    assert_eq!(mappings[0].id, first.id);
    assert_eq!(mappings[0].internal_port, 3000);
    assert_eq!(mappings[1].id, second.id);
    assert_eq!(mappings[1].internal_port, 4000);
}

#[tokio::test]
async fn port_mapping_ticket_exchange_is_one_time_and_mapping_scoped() {
    let (database, organization_id, workspace_id, user_id) = seeded_database().await;
    let mapping = database
        .create_port_mapping(organization_id, workspace_id, 3000, None, user_id, 110)
        .await
        .unwrap();
    let other_mapping = database
        .create_port_mapping(organization_id, workspace_id, 4000, None, user_id, 111)
        .await
        .unwrap();
    let issued = database
        .issue_port_mapping_ticket(&mapping, user_id, 120)
        .await
        .unwrap();
    assert!(!format!("{issued:?}").contains(&issued.ticket));

    assert!(
        database
            .exchange_port_mapping_ticket(
                other_mapping.id,
                &issued.ticket,
                &hash_secret("other-session"),
                121,
                180,
            )
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        database
            .exchange_port_mapping_ticket(
                mapping.id,
                &issued.ticket,
                &hash_secret("session"),
                121,
                180,
            )
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        database
            .exchange_port_mapping_ticket(
                mapping.id,
                &issued.ticket,
                &hash_secret("replayed-session"),
                122,
                180,
            )
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn port_mapping_sessions_are_valid_only_until_expiry() {
    let (database, organization_id, workspace_id, user_id) = seeded_database().await;
    let mapping = database
        .create_port_mapping(organization_id, workspace_id, 3000, None, user_id, 110)
        .await
        .unwrap();
    let issued = database
        .issue_port_mapping_ticket(&mapping, user_id, 120)
        .await
        .unwrap();
    let session_hash = hash_secret("session-cookie-secret");
    database
        .exchange_port_mapping_ticket(mapping.id, &issued.ticket, &session_hash, 121, 180)
        .await
        .unwrap()
        .expect("ticket should create a session");

    assert!(
        database
            .port_mapping_session_valid(mapping.id, &session_hash, 180)
            .await
            .unwrap()
    );
    assert!(
        !database
            .port_mapping_session_valid(mapping.id, &hash_secret("other-cookie"), 180)
            .await
            .unwrap()
    );
    assert!(
        !database
            .port_mapping_session_valid(mapping.id, &session_hash, 181)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn deleting_port_mapping_revokes_its_tickets_and_sessions() {
    let (database, organization_id, workspace_id, user_id) = seeded_database().await;
    let mapping = database
        .create_port_mapping(organization_id, workspace_id, 3000, None, user_id, 110)
        .await
        .unwrap();
    let session_ticket = database
        .issue_port_mapping_ticket(&mapping, user_id, 120)
        .await
        .unwrap();
    let session_hash = hash_secret("session-cookie-secret");
    database
        .exchange_port_mapping_ticket(mapping.id, &session_ticket.ticket, &session_hash, 121, 180)
        .await
        .unwrap()
        .expect("ticket should create a session");
    let unused_ticket = database
        .issue_port_mapping_ticket(&mapping, user_id, 122)
        .await
        .unwrap();

    assert!(
        database
            .delete_port_mapping(workspace_id, mapping.id)
            .await
            .unwrap()
    );
    assert!(
        !database
            .port_mapping_session_valid(mapping.id, &session_hash, 123)
            .await
            .unwrap()
    );
    assert!(
        database
            .exchange_port_mapping_ticket(
                mapping.id,
                &unused_ticket.ticket,
                &hash_secret("new-session"),
                123,
                180,
            )
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        database.get_port_mapping(mapping.id).await,
        Err(StorageError::PortMappingNotFound)
    ));
    assert!(
        !database
            .delete_port_mapping(workspace_id, mapping.id)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn port_mapping_creation_rejects_forbidden_and_invalid_ports() {
    let (database, organization_id, workspace_id, user_id) = seeded_database().await;
    for port in [22, 2222, 8080, 8443] {
        assert!(matches!(
            database
                .create_port_mapping(organization_id, workspace_id, port, None, user_id, 110)
                .await,
            Err(StorageError::InvalidPortMappingPort)
        ));
    }
    assert!(matches!(
        database
            .create_port_mapping(organization_id, workspace_id, 1, None, user_id, 110)
            .await,
        Err(StorageError::InvalidPortMappingPort)
    ));
}
