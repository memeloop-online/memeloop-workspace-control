use memeloop_workspace_control::{
    auth::ApiKeyScope,
    quota::Resources,
    storage::{
        CreateOrganization, CreateWorkspace, CreateWorkspaceTemplate, Database, PutNodePool,
        StorageError,
    },
    templates::{DesktopEndpoint, WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::AccessMode,
};

fn template(name: &str) -> WorkspaceTemplateDocument {
    WorkspaceTemplateDocument::new(
        name,
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
}

#[test]
fn template_names_and_desktop_labels_count_unicode_characters() {
    for character in ["界", "🦀"] {
        let mut document = template(&character.repeat(120));
        document.spec.desktop = Some(DesktopEndpoint {
            internal_port: 6901,
            display_name: Some(character.repeat(80)),
        });
        let yaml = document.to_yaml().unwrap();
        assert_eq!(WorkspaceTemplateDocument::parse(&yaml).unwrap(), document);
        document.metadata.name = character.repeat(121);
        assert!(document.validate().is_err());
        document.metadata.name = character.repeat(120);
        document.spec.desktop.as_mut().unwrap().display_name = Some(character.repeat(81));
        assert!(document.validate().is_err());
    }
}

#[tokio::test]
async fn stored_display_names_accept_character_limits_and_reject_one_extra() {
    let database = Database::connect("sqlite::memory:", "unicode-names".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
        .upsert_image_policy("registry.example/workspace:1", true, 1)
        .await
        .unwrap();
    let user = database
        .create_user_with_initial_key(
            "Admin",
            "admin-token-0000000000000000000000000000",
            true,
            ApiKeyScope::initial_key_defaults(true),
            31_536_000,
            100,
        )
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Development".to_owned(),
                owner_user_id: user.user_id,
            },
            101,
        )
        .await
        .unwrap();
    let stored_template = database
        .create_workspace_template(
            CreateWorkspaceTemplate {
                organization_id: Some(organization.id),
                yaml: template("Development").to_yaml().unwrap(),
            },
            true,
            102,
        )
        .await
        .unwrap();

    for character in ["界", "🦀"] {
        let name = character.repeat(120);
        let too_long = character.repeat(121);
        assert_eq!(
            database
                .update_user(user.user_id, Some(&name), None, None)
                .await
                .unwrap()
                .display_name,
            name
        );
        assert!(matches!(
            database
                .update_user(user.user_id, Some(&too_long), None, None)
                .await,
            Err(StorageError::InvalidUserProfile)
        ));
        assert_eq!(
            database
                .rename_organization(organization.id, &name)
                .await
                .unwrap()
                .name,
            name
        );
        assert!(matches!(
            database
                .rename_organization(organization.id, &too_long)
                .await,
            Err(StorageError::InvalidWorkspace)
        ));
        assert_eq!(
            database
                .get_organization(organization.id)
                .await
                .unwrap()
                .name,
            name
        );
        let pool = PutNodePool {
            display_name: character.repeat(80),
            placement: Default::default(),
            enabled: true,
        };
        assert_eq!(
            database
                .put_node_pool("unicode", &pool, 103)
                .await
                .unwrap()
                .display_name,
            pool.display_name
        );
        let invalid_pool = PutNodePool {
            display_name: character.repeat(81),
            ..pool.clone()
        };
        assert!(matches!(
            database.put_node_pool("unicode", &invalid_pool, 104).await,
            Err(StorageError::InvalidNodePool)
        ));
        let command = CreateWorkspace {
            organization_id: organization.id,
            owner_id: user.user_id,
            name: format!("  {name}  "),
            template_id: stored_template.id,
            resources: None,
            temporary_storage_gib: None,
            use_longhorn: false,
            organization_injection_refs: None,
            user_injection_refs: None,
        };
        assert_eq!(
            database
                .create_workspace(command.clone(), true, user.user_id, 105)
                .await
                .unwrap()
                .name,
            name
        );
        assert!(matches!(
            database
                .create_workspace(
                    CreateWorkspace {
                        name: too_long,
                        ..command
                    },
                    true,
                    user.user_id,
                    106,
                )
                .await,
            Err(StorageError::InvalidWorkspace)
        ));
    }
}
