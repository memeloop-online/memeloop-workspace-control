use memeloop_workspace_control::{
    auth::{ApiKeyScope, Role},
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
    seed_database(database).await
}

async fn seed_database(database: Database) -> (Database, Uuid, Uuid, Uuid) {
    database.migrate().await.unwrap();
    database
        .upsert_image_policy("registry.example/workspace:1", true, 99)
        .await
        .unwrap();

    let user = database
        .create_user_with_initial_key(
            "Port Mapping User",
            USER_TOKEN,
            true,
            memeloop_workspace_control::auth::ApiKeyScope::initial_key_defaults(true),
            31_536_000,
            100,
        )
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
                temporary_storage_gib: None,
                use_longhorn: false,
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
        None,
        user_id,
        110,
    );
    let duplicate = database.create_port_mapping(
        organization_id,
        workspace_id,
        3000,
        Some("other name is ignored"),
        None,
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
            None,
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
async fn desktop_mapping_ensure_is_idempotent() {
    let (database, _organization_id, workspace_id, _user_id) = seeded_database().await;
    let mut workspace = database.get_workspace(workspace_id).await.unwrap();
    workspace.template.desktop = Some(memeloop_workspace_control::templates::DesktopEndpoint {
        internal_port: 6080,
        display_name: Some("Kali desktop".to_owned()),
    });

    let first = database
        .ensure_desktop_port_mapping(&workspace, 110)
        .await
        .unwrap()
        .unwrap();
    let second = database
        .ensure_desktop_port_mapping(&workspace, 111)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(
        database
            .list_port_mappings(workspace_id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn port_mapping_ticket_exchange_is_one_time_and_mapping_scoped() {
    let (database, organization_id, workspace_id, user_id) = seeded_database().await;
    let mapping = database
        .create_port_mapping(
            organization_id,
            workspace_id,
            3000,
            None,
            None,
            user_id,
            110,
        )
        .await
        .unwrap();
    let other_mapping = database
        .create_port_mapping(
            organization_id,
            workspace_id,
            4000,
            None,
            None,
            user_id,
            111,
        )
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
        .create_port_mapping(
            organization_id,
            workspace_id,
            3000,
            None,
            None,
            user_id,
            110,
        )
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
async fn port_mapping_sessions_stop_when_workspace_access_is_revoked() {
    let (database, organization_id, workspace_id, user_id) = seeded_database().await;
    let mapping = database
        .create_port_mapping(
            organization_id,
            workspace_id,
            3000,
            None,
            None,
            user_id,
            110,
        )
        .await
        .unwrap();
    let issued = database
        .issue_port_mapping_ticket(&mapping, user_id, 120)
        .await
        .unwrap();
    let session_hash = hash_secret("revoked-session-cookie");
    database
        .exchange_port_mapping_ticket(mapping.id, &issued.ticket, &session_hash, 121, 180)
        .await
        .unwrap()
        .expect("ticket should create a session");

    let replacement = database
        .create_user_with_initial_key(
            "Replacement administrator",
            "replacement-admin-token-00000000000000000000",
            true,
            ApiKeyScope::initial_key_defaults(true),
            31_536_000,
            122,
        )
        .await
        .unwrap();
    database
        .upsert_membership(
            organization_id,
            replacement.user_id,
            Role::OrganizationAdmin,
            123,
        )
        .await
        .unwrap();
    database
        .update_user(user_id, None, Some(false), None)
        .await
        .unwrap();
    database
        .remove_membership(organization_id, user_id)
        .await
        .unwrap();

    assert!(
        !database
            .port_mapping_session_valid(mapping.id, &session_hash, 124)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn deleting_port_mapping_revokes_its_tickets_and_sessions() {
    let (database, organization_id, workspace_id, user_id) = seeded_database().await;
    let mapping = database
        .create_port_mapping(
            organization_id,
            workspace_id,
            3000,
            None,
            None,
            user_id,
            110,
        )
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
                .create_port_mapping(
                    organization_id,
                    workspace_id,
                    port,
                    None,
                    None,
                    user_id,
                    110
                )
                .await,
            Err(StorageError::InvalidPortMappingPort)
        ));
    }
    assert!(matches!(
        database
            .create_port_mapping(organization_id, workspace_id, 1, None, None, user_id, 110)
            .await,
        Err(StorageError::InvalidPortMappingPort)
    ));
}

#[tokio::test]
async fn updating_port_mapping_preserves_identity_and_revokes_credentials_only_for_port_changes() {
    let (database, organization_id, workspace_id, user_id) = seeded_database().await;
    let original = database
        .create_port_mapping(
            organization_id,
            workspace_id,
            3000,
            Some("网站"),
            None,
            user_id,
            110,
        )
        .await
        .unwrap();
    let session_ticket = database
        .issue_port_mapping_ticket(&original, user_id, 120)
        .await
        .unwrap();
    let cookie = hash_secret("edit-session-cookie");
    assert!(
        database
            .exchange_port_mapping_ticket(original.id, &session_ticket.ticket, &cookie, 121, 180)
            .await
            .unwrap()
            .is_some()
    );
    let pending_ticket = database
        .issue_port_mapping_ticket(&original, user_id, 122)
        .await
        .unwrap();
    let name = "中文".repeat(40);
    let renamed = database
        .update_port_mapping(workspace_id, original.id, 3000, Some(&name), None)
        .await
        .unwrap();
    assert_eq!(renamed.id, original.id);
    assert_eq!(renamed.created_at, original.created_at);
    assert_eq!(renamed.display_name.as_deref(), Some(name.as_str()));
    assert!(
        database
            .port_mapping_session_valid(original.id, &cookie, 123)
            .await
            .unwrap()
    );
    let changed = database
        .update_port_mapping(workspace_id, original.id, 4000, None, None)
        .await
        .unwrap();
    assert_eq!(changed.id, original.id);
    assert_eq!(changed.internal_port, 4000);
    assert_eq!(changed.display_name, None);
    assert!(
        !database
            .port_mapping_session_valid(original.id, &cookie, 124)
            .await
            .unwrap()
    );
    assert!(
        database
            .exchange_port_mapping_ticket(
                original.id,
                &pending_ticket.ticket,
                &hash_secret("new-cookie"),
                124,
                180
            )
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        database
            .list_port_mappings(workspace_id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn conflicting_port_update_rolls_back_revocation_and_keeps_both_mappings() {
    let (database, organization_id, workspace_id, user_id) = seeded_database().await;
    let original = database
        .create_port_mapping(
            organization_id,
            workspace_id,
            3000,
            Some("original"),
            None,
            user_id,
            110,
        )
        .await
        .unwrap();
    database
        .create_port_mapping(
            organization_id,
            workspace_id,
            4000,
            None,
            None,
            user_id,
            110,
        )
        .await
        .unwrap();
    let issued = database
        .issue_port_mapping_ticket(&original, user_id, 120)
        .await
        .unwrap();
    assert!(matches!(
        database
            .update_port_mapping(workspace_id, original.id, 4000, Some("changed"), None)
            .await,
        Err(StorageError::PortMappingPortConflict)
    ));
    assert_eq!(
        database.get_port_mapping(original.id).await.unwrap(),
        original
    );
    assert!(
        database
            .exchange_port_mapping_ticket(
                original.id,
                &issued.ticket,
                &hash_secret("conflict-cookie"),
                121,
                180
            )
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        database
            .list_port_mappings(workspace_id)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn port_mapping_update_is_workspace_scoped_and_validates_new_values() {
    let (database, organization_id, workspace_id, user_id) = seeded_database().await;
    let original = database
        .create_port_mapping(
            organization_id,
            workspace_id,
            3000,
            None,
            None,
            user_id,
            110,
        )
        .await
        .unwrap();
    assert!(matches!(
        database
            .update_port_mapping(Uuid::now_v7(), original.id, 4000, None, None)
            .await,
        Err(StorageError::PortMappingNotFound)
    ));
    assert!(matches!(
        database
            .update_port_mapping(workspace_id, original.id, 3389, None, None)
            .await,
        Err(StorageError::InvalidPortMappingPort)
    ));
    assert!(matches!(
        database
            .update_port_mapping(
                workspace_id,
                original.id,
                4000,
                Some(&"中".repeat(81)),
                None
            )
            .await,
        Err(StorageError::InvalidPortMappingDisplayName)
    ));
    assert_eq!(
        database.get_port_mapping(original.id).await.unwrap(),
        original
    );
}

async fn second_organization_workspace(database: &Database, user_id: Uuid) -> (Uuid, Uuid) {
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Other mapping organization".to_owned(),
                owner_user_id: user_id,
            },
            105,
        )
        .await
        .unwrap();
    let template = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(organization.id),
                yaml: WorkspaceTemplateDocument::new(
                    "Other mapping template",
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
            106,
        )
        .await
        .unwrap();
    let workspace = database
        .create_workspace(
            CreateWorkspace {
                organization_id: organization.id,
                owner_id: user_id,
                name: "other-mapping-workspace".to_owned(),
                template_id: template.id,
                resources: None,
                temporary_storage_gib: None,
                use_longhorn: false,
                organization_injection_refs: None,
                user_injection_refs: None,
            },
            true,
            user_id,
            107,
        )
        .await
        .unwrap();
    (organization.id, workspace.id)
}

async fn exercise_subdomain_ownership(
    database: Database,
    organization_id: Uuid,
    workspace_id: Uuid,
    user_id: Uuid,
) {
    let (other_org, other_workspace) = second_organization_workspace(&database, user_id).await;
    let mapping = database
        .create_port_mapping(
            organization_id,
            workspace_id,
            3000,
            None,
            Some("  Preview  "),
            user_id,
            110,
        )
        .await
        .unwrap();
    assert_eq!(mapping.subdomain, "preview");
    assert_eq!(mapping.hostname("apps.example"), "p-preview.apps.example");
    let session_ticket = database
        .issue_port_mapping_ticket(&mapping, user_id, 120)
        .await
        .unwrap();
    let cookie = hash_secret("hostname-edit-session");
    database
        .exchange_port_mapping_ticket(mapping.id, &session_ticket.ticket, &cookie, 121, 180)
        .await
        .unwrap()
        .unwrap();
    let pending = database
        .issue_port_mapping_ticket(&mapping, user_id, 122)
        .await
        .unwrap();
    let renamed = database
        .update_port_mapping(workspace_id, mapping.id, 3000, None, Some("new-preview"))
        .await
        .unwrap();
    assert_eq!(renamed.id, mapping.id);
    assert_eq!(renamed.created_at, mapping.created_at);
    assert!(
        !database
            .port_mapping_session_valid(mapping.id, &cookie, 123)
            .await
            .unwrap()
    );
    assert!(
        database
            .exchange_port_mapping_ticket(
                mapping.id,
                &pending.ticket,
                &hash_secret("stale-ticket"),
                123,
                180
            )
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        database.get_port_mapping_by_subdomain("preview").await,
        Err(StorageError::PortMappingNotFound)
    ));
    assert_eq!(
        database
            .get_port_mapping_by_subdomain("new-preview")
            .await
            .unwrap()
            .id,
        mapping.id
    );
    // Old gateways can still retain the former route: another sandbox may not claim it.
    assert!(matches!(
        database
            .create_port_mapping(
                other_org,
                other_workspace,
                3000,
                None,
                Some("preview"),
                user_id,
                124
            )
            .await,
        Err(StorageError::PortMappingSubdomainConflict)
    ));
    let old_label_reused = database
        .create_port_mapping(
            organization_id,
            workspace_id,
            4000,
            None,
            Some("preview"),
            user_id,
            125,
        )
        .await
        .unwrap();
    assert_eq!(old_label_reused.subdomain, "preview");
    database
        .delete_port_mapping(workspace_id, old_label_reused.id)
        .await
        .unwrap();
    assert!(matches!(
        database
            .create_port_mapping(
                other_org,
                other_workspace,
                3000,
                None,
                Some("preview"),
                user_id,
                126
            )
            .await,
        Err(StorageError::PortMappingSubdomainConflict)
    ));
    database
        .create_port_mapping(
            organization_id,
            workspace_id,
            4000,
            None,
            Some("preview"),
            user_id,
            127,
        )
        .await
        .unwrap();
    // A rejected label change rolls back ticket revocation and every other edit.
    let ticket = database
        .issue_port_mapping_ticket(&renamed, user_id, 128)
        .await
        .unwrap();
    assert!(matches!(
        database
            .update_port_mapping(
                workspace_id,
                mapping.id,
                3000,
                Some("should roll back"),
                Some("preview")
            )
            .await,
        Err(StorageError::PortMappingSubdomainConflict)
    ));
    assert_eq!(
        database.get_port_mapping(mapping.id).await.unwrap(),
        renamed
    );
    assert!(
        database
            .exchange_port_mapping_ticket(
                mapping.id,
                &ticket.ticket,
                &hash_secret("conflict-survives"),
                129,
                180
            )
            .await
            .unwrap()
            .is_some()
    );
    // Two organizations racing for a fresh label are serialized by database uniqueness.
    let (left, right) = tokio::join!(
        database.create_port_mapping(
            organization_id,
            workspace_id,
            5000,
            None,
            Some("race-label"),
            user_id,
            130
        ),
        database.create_port_mapping(
            other_org,
            other_workspace,
            5000,
            None,
            Some("race-label"),
            user_id,
            130
        ),
    );
    let results = [left, right];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(StorageError::PortMappingSubdomainConflict)))
            .count(),
        1
    );
}

#[tokio::test]
async fn custom_subdomain_rename_reuse_conflict_and_revocation_are_atomic() {
    let (database, organization, workspace, user) = seeded_database().await;
    exercise_subdomain_ownership(database, organization, workspace, user).await;
}

#[tokio::test]
async fn subdomains_are_normalized_validated_and_generated_short() {
    let (database, organization, workspace, user) = seeded_database().await;
    let generated = database
        .create_port_mapping(organization, workspace, 3000, None, None, user, 110)
        .await
        .unwrap();
    assert_eq!(generated.subdomain.len(), 8);
    assert!(
        generated
            .subdomain
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
    let other = database
        .create_port_mapping(organization, workspace, 4000, None, None, user, 110)
        .await
        .unwrap();
    assert_ne!(generated.subdomain, other.subdomain);
    let unchanged = database
        .update_port_mapping(workspace, generated.id, 3000, Some("display edit"), None)
        .await
        .unwrap();
    assert_eq!(unchanged.subdomain, generated.subdomain);
    for invalid in [
        "",
        " ",
        "-leading",
        "trailing-",
        "nested.label",
        "space here",
        "under_score",
        "*",
        "中文",
        "a\nhost",
    ] {
        assert!(
            matches!(
                database
                    .create_port_mapping(
                        organization,
                        workspace,
                        5000,
                        None,
                        Some(invalid),
                        user,
                        110
                    )
                    .await,
                Err(StorageError::InvalidPortMappingSubdomain)
            ),
            "{invalid:?}"
        );
    }
    let longest = "a".repeat(61);
    let accepted = database
        .create_port_mapping(
            organization,
            workspace,
            5000,
            None,
            Some(&longest),
            user,
            110,
        )
        .await
        .unwrap();
    assert_eq!(accepted.subdomain, longest);
    assert!(matches!(
        database
            .update_port_mapping(workspace, accepted.id, 5000, None, Some(&"a".repeat(62)))
            .await,
        Err(StorageError::InvalidPortMappingSubdomain)
    ));
    let snapshot = database.export_snapshot(140).await.unwrap();
    assert!(
        snapshot.tables["workspace_port_mappings"]
            .iter()
            .any(|row| row["subdomain"] == generated.subdomain)
    );
    assert!(
        snapshot.tables["workspace_port_subdomain_claims"]
            .iter()
            .any(|row| row["subdomain"] == generated.subdomain
                && row["workspace_id"] == workspace.to_string())
    );
}

#[tokio::test]
async fn postgres_subdomain_ownership_races_and_revocation_when_configured() {
    let Ok(url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        return;
    };
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    let schema = format!("mwc_subdomains_{}", Uuid::now_v7().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let mut scoped = url::Url::parse(&url).unwrap();
    scoped
        .query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema}"));
    let database = Database::connect(scoped.as_str(), "port-mapping-test".parse().unwrap())
        .await
        .unwrap();
    let (database, organization, workspace, user) = seed_database(database).await;
    exercise_subdomain_ownership(database, organization, workspace, user).await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
