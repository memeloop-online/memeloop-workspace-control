use memeloop_workspace_control::{
    auth::Role,
    config::InstallationId,
    storage::{CreateOrganization, Database, StorageError},
};
use uuid::Uuid;

async fn database(name: &str) -> Database {
    let installation_id: InstallationId = name.parse().unwrap();
    let database = Database::connect("sqlite::memory:", installation_id)
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
}

#[tokio::test]
async fn disabling_the_last_active_organization_admin_is_rejected() {
    let database = database("disable-last-org").await;
    let owner = database
        .create_user(
            "Owner",
            "owner-disable-org-admin-token-000000000000000",
            false,
            1,
        )
        .await
        .unwrap();
    let inactive_admin = database
        .create_user(
            "Inactive administrator",
            "inactive-disable-org-admin-token-000000000000",
            false,
            2,
        )
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Guarded organization".to_owned(),
                owner_user_id: owner.user_id,
            },
            3,
        )
        .await
        .unwrap();
    database
        .upsert_membership(
            organization.id,
            inactive_admin.user_id,
            Role::OrganizationAdmin,
            4,
        )
        .await
        .unwrap();
    database
        .update_user(inactive_admin.user_id, None, None, Some(true))
        .await
        .unwrap();

    assert!(matches!(
        database
            .update_user(owner.user_id, None, None, Some(true))
            .await,
        Err(StorageError::LastOrganizationAdmin)
    ));
}

#[tokio::test]
async fn concurrent_disables_leave_an_active_organization_admin() {
    let database = database("disable-org-race").await;
    let first = database
        .create_user(
            "First",
            "first-disable-org-admin-token-000000000000000",
            false,
            1,
        )
        .await
        .unwrap();
    let second = database
        .create_user(
            "Second",
            "second-disable-org-admin-token-00000000000000",
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
        database.update_user(first.user_id, None, None, Some(true)),
        database.update_user(second.user_id, None, None, Some(true)),
    );
    assert!(matches!(
        [first_result, second_result],
        [Ok(_), Err(StorageError::LastOrganizationAdmin)]
            | [Err(StorageError::LastOrganizationAdmin), Ok(_)]
    ));

    let members = database
        .list_members_page(organization.id, None, None, None)
        .await
        .unwrap();
    assert_eq!(
        members
            .items
            .iter()
            .filter(|member| member.role == Role::OrganizationAdmin && !member.user.disabled)
            .count(),
        1
    );
}

#[tokio::test]
async fn disabled_administrators_do_not_allow_removing_the_only_active_administrator() {
    let database = database("active-admin-member").await;
    let active = database
        .create_user(
            "Active",
            "active-org-admin-token-000000000000000000000",
            false,
            1,
        )
        .await
        .unwrap();
    let first_disabled = database
        .create_user(
            "First disabled",
            "first-disabled-org-admin-token-000000000000000",
            false,
            2,
        )
        .await
        .unwrap();
    let second_disabled = database
        .create_user(
            "Second disabled",
            "second-disabled-org-admin-token-00000000000000",
            false,
            3,
        )
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Active administrator guard".to_owned(),
                owner_user_id: active.user_id,
            },
            4,
        )
        .await
        .unwrap();
    for (user_id, now) in [(first_disabled.user_id, 5), (second_disabled.user_id, 6)] {
        database
            .upsert_membership(organization.id, user_id, Role::OrganizationAdmin, now)
            .await
            .unwrap();
        database
            .update_user(user_id, None, None, Some(true))
            .await
            .unwrap();
    }

    assert!(matches!(
        database
            .upsert_membership(organization.id, active.user_id, Role::Member, 7)
            .await,
        Err(StorageError::LastOrganizationAdmin)
    ));
    assert!(matches!(
        database
            .remove_membership(organization.id, active.user_id)
            .await,
        Err(StorageError::LastOrganizationAdmin)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn postgres_serializes_disable_and_membership_demotion() {
    let Ok(database_url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        eprintln!("skipping PostgreSQL administrator guard test: MWC_TEST_POSTGRES_URL is not set");
        return;
    };
    let schema = format!("mwc_admin_guard_{}", Uuid::now_v7().simple());
    let administration = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .unwrap();
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&administration)
        .await
        .unwrap();
    let mut scoped_url = url::Url::parse(&database_url).unwrap();
    scoped_url
        .query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema}"));
    let database = Database::connect(scoped_url.as_str(), "pg-admin-guard".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();

    let first = database
        .create_user(
            "First",
            "first-postgres-admin-guard-token-000000000000",
            false,
            1,
        )
        .await
        .unwrap();
    let second = database
        .create_user(
            "Second",
            "second-postgres-admin-guard-token-00000000000",
            false,
            2,
        )
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "PostgreSQL administrator guard".to_owned(),
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

    let (disable, demote) = tokio::join!(
        database.update_user(first.user_id, None, None, Some(true)),
        database.upsert_membership(organization.id, second.user_id, Role::Member, 5),
    );
    assert!(
        (disable.is_ok() && matches!(demote, Err(StorageError::LastOrganizationAdmin)))
            || (matches!(disable, Err(StorageError::LastOrganizationAdmin)) && demote.is_ok()),
        "unexpected PostgreSQL disable/demotion outcomes: disable={disable:?}, demote={demote:?}"
    );
    let members = database
        .list_members_page(organization.id, None, None, None)
        .await
        .unwrap();
    assert_eq!(
        members
            .items
            .iter()
            .filter(|member| member.role == Role::OrganizationAdmin && !member.user.disabled)
            .count(),
        1
    );

    drop(database);
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&administration)
        .await
        .unwrap();
    administration.close().await;
}
