use crate::{
    auth::{ApiKeyScope, Role},
    storage::{CreateOrganization, DatabaseSnapshot, OidcBindingResult},
};
use uuid::Uuid;

use super::*;

const ISSUER: &str = "https://login.example/oidc/dev/";
const SUBJECT: &str = "pairwise-identity-for-owner";
const OWNER_TOKEN: &str = "fixture-owner-api-key-that-is-not-a-real-secret";
const OTHER_TOKEN: &str = "fixture-another-api-key-that-is-not-a-real-secret";
const SESSION: &str = "abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz12";

async fn assert_v32_upgrade_and_oidc(database: &Database) -> Option<DatabaseSnapshot> {
    let now = unix_timestamp().unwrap();
    let owner = database
        .create_user_with_initial_key(
            "existing account",
            OWNER_TOKEN,
            false,
            [
                ApiKeyScope::initial_key_defaults(false),
                vec![
                    ApiKeyScope::ManageOrganization,
                    ApiKeyScope::ManageMembers,
                    ApiKeyScope::ManageLockedInjections,
                    ApiKeyScope::DeleteWorkspace,
                ],
            ]
            .concat(),
            now + 3600,
            now,
        )
        .await
        .unwrap();
    let other = database
        .create_user_with_initial_key(
            "existing account",
            OTHER_TOKEN,
            false,
            ApiKeyScope::initial_key_defaults(false),
            now + 3600,
            now,
        )
        .await
        .unwrap();
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "verified existing organization".into(),
                owner_user_id: owner.user_id,
            },
            now,
        )
        .await
        .unwrap();

    database.migrate().await.unwrap();
    database.migrate().await.unwrap();
    assert_eq!(
        database.schema_version().await.unwrap(),
        schema::SCHEMA_VERSION,
    );
    let cli = database.authenticate(OWNER_TOKEN).await.unwrap().unwrap();
    assert_eq!(cli.user_id, owner.user_id);
    assert_eq!(cli.memberships[0].organization_id, organization.id);
    assert_eq!(cli.memberships[0].role, Role::OrganizationAdmin);
    assert!(cli.may_manage_api_keys());

    assert!(
        !database
            .oidc_create_session(ISSUER, SUBJECT, SESSION, now + 900)
            .await
            .unwrap()
    );
    let delegated = database
        .create_api_key(
            owner.user_id,
            "Initial key",
            ApiKeyScope::initial_key_defaults(false),
            Some(now + 3600),
            None,
            now,
        )
        .await
        .unwrap();
    assert_eq!(database.oidc_owner(&delegated.token).await.unwrap(), None);
    assert_eq!(
        database
            .oidc_bind(&delegated.token, ISSUER, SUBJECT, now)
            .await
            .unwrap(),
        OidcBindingResult::Unauthorized,
    );
    let allowlisted = database
        .create_api_key(
            owner.user_id,
            "Allowlisted full scopes",
            cli.api_key_scopes.clone(),
            Some(now + 3600),
            Some(vec![Uuid::now_v7()]),
            now,
        )
        .await
        .unwrap();
    assert_eq!(database.oidc_owner(&allowlisted.token).await.unwrap(), None);
    let rotated = database
        .create_api_key(
            owner.user_id,
            "Rotated full account access",
            cli.api_key_scopes.clone(),
            Some(now + 3600),
            None,
            now,
        )
        .await
        .unwrap();
    assert_eq!(
        database.oidc_owner(&rotated.token).await.unwrap(),
        Some(owner.user_id),
    );
    assert_eq!(
        database
            .oidc_bind(&rotated.token, ISSUER, SUBJECT, now)
            .await
            .unwrap(),
        OidcBindingResult::Changed,
    );
    for (token, subject) in [(OWNER_TOKEN, "other-subject"), (OTHER_TOKEN, SUBJECT)] {
        assert_eq!(
            database
                .oidc_bind(token, ISSUER, subject, now)
                .await
                .unwrap(),
            OidcBindingResult::Conflict,
        );
    }
    assert!(
        database
            .oidc_create_session(ISSUER, SUBJECT, SESSION, now + 900)
            .await
            .unwrap()
    );
    let browser = database
        .oidc_session_principal(SESSION, ISSUER, now)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(browser.user_id, cli.user_id);
    assert_eq!(browser.memberships, cli.memberships);
    assert!(
        browser
            .api_key_scopes
            .contains(&ApiKeyScope::ManageOrganization)
    );
    assert!(
        !browser
            .api_key_scopes
            .contains(&ApiKeyScope::DeleteWorkspace)
    );
    assert!(!browser.may_manage_api_keys());
    assert!(
        database
            .oidc_session_principal(SESSION, ISSUER, now + 900)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        database
            .oidc_session_principal(SESSION, "https://later.example", now)
            .await
            .unwrap()
            .is_none()
    );

    let flow = "flow-once";
    database
        .oidc_put_flow(flow, "public-state and verifier", now + 60, now)
        .await
        .unwrap();
    let (first, second) = tokio::join!(
        database.oidc_take_flow(flow, now),
        database.oidc_take_flow(flow, now)
    );
    let taken = [first.unwrap(), second.unwrap()];
    assert_eq!(
        taken
            .iter()
            .filter_map(Option::as_deref)
            .collect::<Vec<_>>(),
        vec!["public-state and verifier"]
    );
    database
        .oidc_put_flow("expired-flow", "expired", now + 1, now)
        .await
        .unwrap();
    assert_eq!(
        database
            .oidc_take_flow("expired-flow", now + 1)
            .await
            .unwrap(),
        None
    );
    database
        .oidc_put_flow("left-over", "ephemeral", now + 60, now + 1)
        .await
        .unwrap();

    assert_eq!(
        database
            .oidc_unbind(OTHER_TOKEN, ISSUER, SUBJECT, now)
            .await
            .unwrap(),
        OidcBindingResult::Conflict,
    );
    for wrong_subject in ["other-subject", "existing account"] {
        assert_eq!(
            database
                .oidc_unbind(OWNER_TOKEN, ISSUER, wrong_subject, now)
                .await
                .unwrap(),
            OidcBindingResult::Conflict,
        );
    }
    let snapshot = if let Database::Sqlite { .. } = database {
        Some(database.export_snapshot(now).await.unwrap())
    } else {
        None
    };
    assert_eq!(
        database
            .oidc_unbind(OWNER_TOKEN, ISSUER, SUBJECT, now)
            .await
            .unwrap(),
        OidcBindingResult::Changed,
    );
    assert!(
        database
            .oidc_session_principal(SESSION, ISSUER, now)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        database
            .oidc_bind(OTHER_TOKEN, ISSUER, SUBJECT, now)
            .await
            .unwrap(),
        OidcBindingResult::Changed,
    );
    assert!(
        database
            .oidc_session_principal(SESSION, ISSUER, now)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        database
            .oidc_create_session(ISSUER, SUBJECT, SESSION, now + 900)
            .await
            .unwrap()
    );

    database
        .update_user(other.user_id, None, None, Some(true))
        .await
        .unwrap();
    assert!(
        database
            .oidc_session_principal(SESSION, ISSUER, now)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(database.oidc_owner(OTHER_TOKEN).await.unwrap(), None);
    assert!(
        !database
            .oidc_create_session(ISSUER, SUBJECT, "new", now + 900)
            .await
            .unwrap()
    );
    database
        .update_user(other.user_id, None, None, Some(false))
        .await
        .unwrap();
    assert!(
        database
            .oidc_session_principal(SESSION, ISSUER, now)
            .await
            .unwrap()
            .is_none()
    );

    assert!(
        database
            .oidc_create_session(ISSUER, SUBJECT, SESSION, now + 900)
            .await
            .unwrap()
    );
    database.oidc_revoke_session(SESSION).await.unwrap();
    assert!(
        database
            .oidc_session_principal(SESSION, ISSUER, now)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        database
            .oidc_unbind(OTHER_TOKEN, ISSUER, SUBJECT, now + 3600)
            .await
            .unwrap(),
        OidcBindingResult::Unauthorized,
    );

    let audit = database
        .page_audit(crate::storage::AuditFilter {
            organization_id: None,
            limit: 10,
            offset: 0,
            action: Some("user.oidc.unbind".into()),
            actor: None,
            workspace: None,
            query: None,
        })
        .await
        .unwrap();
    assert_eq!(audit.items.len(), 1);
    assert_eq!(audit.items[0].actor_user_id, Some(owner.user_id));
    assert_eq!(audit.items[0].metadata["issuer"], ISSUER);
    assert!(audit.items[0].metadata["subject_sha256"].as_str().is_some());
    assert_eq!(
        database.authenticate(OWNER_TOKEN).await.unwrap().unwrap(),
        cli
    );
    snapshot
}

#[tokio::test]
async fn sqlite_v32_new_oidc_does_not_remap_users_and_preserves_cli_access() {
    let database = Database::connect("sqlite::memory:", "schema-pg".parse().unwrap())
        .await
        .unwrap();
    let Database::Sqlite { pool, .. } = &database else {
        unreachable!()
    };
    sqlx::query(schema::MIGRATION_TABLE)
        .execute(pool)
        .await
        .unwrap();
    for statement in schema::BASELINE {
        sqlx::query(statement).execute(pool).await.unwrap();
    }
    sqlx::query("INSERT INTO schema_migrations VALUES (32, 5)")
        .execute(pool)
        .await
        .unwrap();
    let snapshot = assert_v32_upgrade_and_oidc(&database).await.unwrap();
    assert_eq!(snapshot.tables["oidc_identities"].len(), 1);
    assert_eq!(snapshot.tables["oidc_identities"][0]["subject"], SUBJECT);
    assert!(!snapshot.tables.contains_key("oidc_sessions"));
    assert!(!snapshot.tables.contains_key("oidc_flows"));

    if let Ok(url) = std::env::var("MWC_TEST_POSTGRES_URL") {
        let name = format!("mwc_oidc_snapshot_{}", Uuid::now_v7().simple());
        let target = postgres_database(&url, &name, None).await;
        target.migrate().await.unwrap();
        target.import_snapshot(&snapshot).await.unwrap();
        let now = unix_timestamp().unwrap();
        let owner = target.authenticate(OWNER_TOKEN).await.unwrap().unwrap();
        assert!(
            target
                .oidc_session_principal(SESSION, ISSUER, now)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(target.oidc_take_flow("left-over", now).await.unwrap(), None);
        assert!(
            target
                .oidc_create_session(ISSUER, SUBJECT, SESSION, now + 900)
                .await
                .unwrap()
        );
        assert_eq!(
            target
                .oidc_session_principal(SESSION, ISSUER, now)
                .await
                .unwrap()
                .unwrap()
                .user_id,
            owner.user_id,
        );
        drop_schema(target, &name).await;
    }
}

#[tokio::test]
async fn postgres_v32_new_oidc_does_not_remap_users_and_preserves_cli_access() {
    let Ok(url) = std::env::var("MWC_TEST_POSTGRES_URL") else {
        return;
    };
    let name = format!("mwc_oidc_upgrade_{}", Uuid::now_v7().simple());
    let database = postgres_database(&url, &name, Some(32)).await;
    let Database::Postgres { pool, .. } = &database else {
        unreachable!()
    };
    for statement in schema::BASELINE {
        sqlx::query(statement).execute(pool).await.unwrap();
    }
    assert!(assert_v32_upgrade_and_oidc(&database).await.is_none());
    drop_schema(database, &name).await;
}

async fn drop_schema(database: Database, name: &str) {
    let Database::Postgres { pool, .. } = database else {
        unreachable!()
    };
    sqlx::query(&format!("DROP SCHEMA {name} CASCADE"))
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}
