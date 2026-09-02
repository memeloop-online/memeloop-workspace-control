use memeloop_workspace_control::{
    auth::Role,
    config::InstallationId,
    storage::{CreateOrganization, Database, StorageError},
};

#[tokio::test]
async fn last_organization_admin_cannot_be_demoted_or_removed() {
    let installation_id: InstallationId = "membership-guards".parse().unwrap();
    let database = Database::connect("sqlite::memory:", installation_id)
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let owner = database
        .create_user(
            "Owner",
            "owner-membership-guard-token-000000000000000000",
            false,
            1,
        )
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Guarded organization".to_owned(),
                owner_user_id: owner.user_id,
            },
            2,
        )
        .await
        .unwrap();

    assert!(matches!(
        database
            .upsert_membership(organization.id, owner.user_id, Role::Member, 3)
            .await,
        Err(StorageError::LastOrganizationAdmin)
    ));
    assert!(matches!(
        database
            .remove_membership(organization.id, owner.user_id)
            .await,
        Err(StorageError::LastOrganizationAdmin)
    ));
}

#[tokio::test]
async fn concurrent_admin_demotions_leave_one_administrator() {
    let installation_id: InstallationId = "membership-concur".parse().unwrap();
    let database = Database::connect("sqlite::memory:", installation_id)
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let first = database
        .create_user(
            "First",
            "first-membership-guard-token-000000000000000",
            false,
            1,
        )
        .await
        .unwrap();
    let second = database
        .create_user(
            "Second",
            "second-membership-guard-token-000000000000000",
            false,
            2,
        )
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Concurrent organization".to_owned(),
                owner_user_id: first.user_id,
            },
            3,
        )
        .await
        .unwrap();
    database
        .upsert_membership(organization.id, second.user_id, Role::OrganizationAdmin, 4)
        .await
        .unwrap();

    let (first_result, second_result) = tokio::join!(
        database.upsert_membership(organization.id, first.user_id, Role::Member, 5),
        database.upsert_membership(organization.id, second.user_id, Role::Member, 5),
    );
    assert!(matches!(
        [first_result, second_result],
        [Ok(()), Err(StorageError::LastOrganizationAdmin)]
            | [Err(StorageError::LastOrganizationAdmin), Ok(())]
    ));

    let members = database
        .list_members_page(organization.id, None, None, None)
        .await
        .unwrap();
    assert_eq!(
        members
            .items
            .iter()
            .filter(|member| member.role == Role::OrganizationAdmin)
            .count(),
        1
    );
}
