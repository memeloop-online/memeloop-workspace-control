#[path = "ttyd_mtls_harness.rs"]
mod harness;
#[path = "ttyd_mtls_support.rs"]
mod support;

use super::super::{
    OWNER_INSTALLATION_LABEL,
    client::{DeleteProgress, ReconcileError},
};
use harness::{FakeKube, FakeKubeConfig, RecordedRequest};
use support::{
    CLIENT_SECRET_NAME, FILTER_UID, HIGRESS_NAMESPACE, INSTALLATION_ID, ReconcileErrorKind,
    SecretFailure, WEB_SHELL_DOMAIN, assert_reconcile_error, builder, client_secret_path,
    companion_secret_path, envoy_filter, filter_path, foreign_labels, ingress, ingress_path,
    owned_labels, secret_case, valid_client_secret, valid_companion_secret, workspace,
};

#[tokio::test]
async fn disabled_reconcile_and_delete_never_call_envoyfilter() {
    let workspace = workspace();
    let builder = builder(false, Some(WEB_SHELL_DOMAIN));
    let fake = FakeKube::new(
        &builder,
        &workspace,
        FakeKubeConfig::default().namespace(true),
    );
    let coordinator = fake.coordinator(builder);

    coordinator.reconcile(&workspace).await.unwrap();
    coordinator.delete_or_confirm(&workspace).await.unwrap();

    assert!(
        fake.requests()
            .iter()
            .all(|request| !request.uri.contains("/envoyfilters/")),
        "mTLS-disabled lifecycle must not call the EnvoyFilter API"
    );
}

#[tokio::test]
async fn mtls_secret_validation_precedes_filter_and_ingress_patches() {
    let cases = [
        (
            SecretFailure::MissingClient,
            ReconcileErrorKind::MissingClient,
        ),
        (
            SecretFailure::EmptyClient,
            ReconcileErrorKind::InvalidClient,
        ),
        (
            SecretFailure::MissingCompanion,
            ReconcileErrorKind::MissingCompanion,
        ),
        (
            SecretFailure::EmptyCompanion,
            ReconcileErrorKind::InvalidCompanion,
        ),
    ];

    for (failure, expected) in cases {
        let workspace = workspace();
        let builder = builder(true, Some(WEB_SHELL_DOMAIN));
        let (client_secret, companion_secret) = secret_case(&builder, failure);
        let fake = FakeKube::new(
            &builder,
            &workspace,
            FakeKubeConfig::default()
                .namespace(true)
                .client_secret(client_secret)
                .companion_secret(companion_secret),
        );
        let coordinator = fake.coordinator(builder.clone());

        let result = coordinator.reconcile(&workspace).await;
        assert_reconcile_error(result, expected);

        let names = builder.runtime_names(&workspace).unwrap();
        let filter_path = filter_path(&builder, &names.resources.web_shell_envoy_filter);
        let ingress_path = ingress_path(&names.resources.web_shell_ingress);
        assert_no_patch(&fake.requests(), &filter_path);
        assert_no_patch(&fake.requests(), &ingress_path);
    }
}

#[tokio::test]
async fn enabled_reconcile_checks_secrets_and_owned_filter_before_ingress() {
    let workspace = workspace();
    let builder = builder(true, Some(WEB_SHELL_DOMAIN));
    let names = builder.runtime_names(&workspace).unwrap();
    let owned_filter = envoy_filter(&builder, &workspace, owned_labels(&builder, &workspace));
    let fake = FakeKube::new(
        &builder,
        &workspace,
        FakeKubeConfig::default()
            .namespace(true)
            .filter(Some(owned_filter))
            .client_secret(Some(valid_client_secret(&builder)))
            .companion_secret(Some(valid_companion_secret(&builder))),
    );
    let coordinator = fake.coordinator(builder.clone());

    coordinator.reconcile(&workspace).await.unwrap();

    let requests = fake.requests();
    let client_secret_get = request_index(&requests, "GET", &client_secret_path(&builder));
    let companion_secret_get = request_index(&requests, "GET", &companion_secret_path(&builder));
    let filter_get = request_index(
        &requests,
        "GET",
        &filter_path(&builder, &names.resources.web_shell_envoy_filter),
    );
    let filter_patch = request_index(
        &requests,
        "PATCH",
        &filter_path(&builder, &names.resources.web_shell_envoy_filter),
    );
    let ingress_get = request_index(
        &requests,
        "GET",
        &ingress_path(&names.resources.web_shell_ingress),
    );
    let ingress_patch = request_index(
        &requests,
        "PATCH",
        &ingress_path(&names.resources.web_shell_ingress),
    );
    assert!(client_secret_get < companion_secret_get);
    assert!(companion_secret_get < filter_get);
    assert!(filter_get < filter_patch);
    assert!(filter_patch < ingress_get);
    assert!(ingress_get < ingress_patch);

    let filter_body = requests[filter_patch].body.as_ref().unwrap();
    assert_eq!(
        filter_body["metadata"]["name"],
        names.resources.web_shell_envoy_filter
    );
    assert_eq!(
        filter_body["metadata"]["labels"][OWNER_INSTALLATION_LABEL],
        INSTALLATION_ID
    );
    assert!(filter_body["spec"]["configPatches"].is_array());
    let ingress_body = requests[ingress_patch].body.as_ref().unwrap();
    assert_eq!(
        ingress_body["metadata"]["annotations"]["nginx.ingress.kubernetes.io/proxy-ssl-secret"],
        format!("{HIGRESS_NAMESPACE}/{CLIENT_SECRET_NAME}")
    );
}

#[tokio::test]
async fn foreign_envoy_filter_ownership_blocks_reconcile_patch() {
    let workspace = workspace();
    let builder = builder(true, Some(WEB_SHELL_DOMAIN));
    let names = builder.runtime_names(&workspace).unwrap();
    let foreign_filter = envoy_filter(&builder, &workspace, foreign_labels(&workspace));
    let fake = FakeKube::new(
        &builder,
        &workspace,
        FakeKubeConfig::default()
            .namespace(true)
            .filter(Some(foreign_filter))
            .client_secret(Some(valid_client_secret(&builder)))
            .companion_secret(Some(valid_companion_secret(&builder))),
    );
    let coordinator = fake.coordinator(builder.clone());

    assert!(matches!(
        coordinator.reconcile(&workspace).await,
        Err(ReconcileError::Ownership(_))
    ));
    let requests = fake.requests();
    assert_no_patch(
        &requests,
        &filter_path(&builder, &names.resources.web_shell_envoy_filter),
    );
    assert_no_patch(&requests, &ingress_path(&names.resources.web_shell_ingress));
}

#[tokio::test]
async fn foreign_envoy_filter_ownership_blocks_delete() {
    let workspace = workspace();
    let builder = builder(true, None);
    let names = builder.runtime_names(&workspace).unwrap();
    let foreign_filter = envoy_filter(&builder, &workspace, foreign_labels(&workspace));
    let fake = FakeKube::new(
        &builder,
        &workspace,
        FakeKubeConfig::default()
            .namespace(true)
            .filter(Some(foreign_filter)),
    );
    let coordinator = fake.coordinator(builder.clone());

    assert!(matches!(
        coordinator.delete_or_confirm(&workspace).await,
        Err(ReconcileError::Ownership(_))
    ));
    assert_no_delete(
        &fake.requests(),
        &filter_path(&builder, &names.resources.web_shell_envoy_filter),
    );
}

#[tokio::test]
async fn delete_rejects_owned_filter_without_uid() {
    let workspace = workspace();
    let builder = builder(true, None);
    let names = builder.runtime_names(&workspace).unwrap();
    let mut filter = envoy_filter(&builder, &workspace, owned_labels(&builder, &workspace));
    filter["metadata"].as_object_mut().unwrap().remove("uid");
    let fake = FakeKube::new(
        &builder,
        &workspace,
        FakeKubeConfig::default()
            .namespace(true)
            .filter(Some(filter)),
    );
    let coordinator = fake.coordinator(builder.clone());

    assert!(matches!(
        coordinator.delete_or_confirm(&workspace).await,
        Err(ReconcileError::MissingEnvoyFilterUid)
    ));
    assert_no_delete(
        &fake.requests(),
        &filter_path(&builder, &names.resources.web_shell_envoy_filter),
    );
}

#[tokio::test]
async fn delete_uses_filter_uid_after_ingress_disappears() {
    let workspace = workspace();
    let builder = builder(true, Some(WEB_SHELL_DOMAIN));
    let names = builder.runtime_names(&workspace).unwrap();
    let fake = FakeKube::new(
        &builder,
        &workspace,
        FakeKubeConfig::default()
            .namespace(true)
            .ingress(Some(ingress(
                &builder,
                &workspace,
                owned_labels(&builder, &workspace),
                false,
            )))
            .filter(Some(envoy_filter(
                &builder,
                &workspace,
                owned_labels(&builder, &workspace),
            ))),
    );
    let coordinator = fake.coordinator(builder.clone());

    assert_eq!(
        coordinator.delete_or_confirm(&workspace).await.unwrap(),
        DeleteProgress::DeletionRequested
    );
    let first_requests = fake.requests();
    let ingress_delete = request_index(
        &first_requests,
        "DELETE",
        &ingress_path(&names.resources.web_shell_ingress),
    );
    assert_no_delete(
        &first_requests[..=ingress_delete],
        &filter_path(&builder, &names.resources.web_shell_envoy_filter),
    );

    assert_eq!(
        coordinator.delete_or_confirm(&workspace).await.unwrap(),
        DeleteProgress::DeletionRequested
    );
    let second_requests = fake.requests();
    let filter_path = filter_path(&builder, &names.resources.web_shell_envoy_filter);
    let filter_get = request_index(&second_requests, "GET", &filter_path);
    let filter_delete = request_index(&second_requests, "DELETE", &filter_path);
    assert!(ingress_delete < filter_get);
    assert!(filter_get < filter_delete);
    assert_eq!(
        second_requests[filter_delete].body.as_ref().unwrap()["preconditions"]["uid"],
        FILTER_UID
    );

    assert_eq!(
        coordinator.delete_or_confirm(&workspace).await.unwrap(),
        DeleteProgress::Gone
    );
}

#[tokio::test]
async fn draining_web_shell_waits_for_ingress_absence_before_filter_delete() {
    let workspace = workspace();
    let builder = builder(true, None);
    let names = builder.runtime_names(&workspace).unwrap();
    let fake = FakeKube::new(
        &builder,
        &workspace,
        FakeKubeConfig::default()
            .namespace(true)
            .ingress(Some(ingress(
                &builder,
                &workspace,
                owned_labels(&builder, &workspace),
                true,
            )))
            .filter(Some(envoy_filter(
                &builder,
                &workspace,
                owned_labels(&builder, &workspace),
            )))
            .retain_ingress_on_delete(true),
    );
    let coordinator = fake.coordinator(builder.clone());

    assert!(matches!(
        coordinator.reconcile(&workspace).await,
        Err(ReconcileError::WebShellIngressTerminating)
    ));
    let first_requests = fake.requests();
    let ingress_delete = request_index(
        &first_requests,
        "DELETE",
        &ingress_path(&names.resources.web_shell_ingress),
    );
    assert_no_delete(
        &first_requests,
        &filter_path(&builder, &names.resources.web_shell_envoy_filter),
    );

    assert!(matches!(
        coordinator.reconcile(&workspace).await,
        Err(ReconcileError::WebShellIngressTerminating)
    ));
    let second_requests = fake.requests();
    assert_no_delete(
        &second_requests,
        &filter_path(&builder, &names.resources.web_shell_envoy_filter),
    );

    fake.set_ingress(None);
    coordinator.reconcile(&workspace).await.unwrap();
    let requests = fake.requests();
    let filter_delete = request_index(
        &requests,
        "DELETE",
        &filter_path(&builder, &names.resources.web_shell_envoy_filter),
    );
    assert!(ingress_delete < filter_delete);
    assert_eq!(
        requests[filter_delete].body.as_ref().unwrap()["preconditions"]["uid"],
        FILTER_UID
    );
}

#[tokio::test]
async fn missing_workspace_namespace_still_deletes_owned_filter_before_gone() {
    let workspace = workspace();
    let builder = builder(true, None);
    let names = builder.runtime_names(&workspace).unwrap();
    let filter_path = filter_path(&builder, &names.resources.web_shell_envoy_filter);
    let fake = FakeKube::new(
        &builder,
        &workspace,
        FakeKubeConfig::default().filter(Some(envoy_filter(
            &builder,
            &workspace,
            owned_labels(&builder, &workspace),
        ))),
    );
    let coordinator = fake.coordinator(builder.clone());

    assert_eq!(
        coordinator.delete_or_confirm(&workspace).await.unwrap(),
        DeleteProgress::DeletionRequested
    );
    let first_requests = fake.requests();
    let filter_delete = request_index(&first_requests, "DELETE", &filter_path);
    assert_eq!(
        first_requests[filter_delete].body.as_ref().unwrap()["preconditions"]["uid"],
        FILTER_UID
    );

    assert_eq!(
        coordinator.delete_or_confirm(&workspace).await.unwrap(),
        DeleteProgress::Gone
    );
}

fn request_index(requests: &[RecordedRequest], method: &str, path: &str) -> usize {
    requests
        .iter()
        .position(|request| request.method == method && request.uri.split('?').next() == Some(path))
        .unwrap_or_else(|| panic!("missing {method} {path}; requests: {requests:?}"))
}

fn assert_no_patch(requests: &[RecordedRequest], path: &str) {
    assert!(
        requests.iter().all(|request| {
            request.method != "PATCH" || request.uri.split('?').next() != Some(path)
        }),
        "unexpected PATCH {path}; requests: {requests:?}"
    );
}

fn assert_no_delete(requests: &[RecordedRequest], path: &str) {
    assert!(
        requests.iter().all(|request| {
            request.method != "DELETE" || request.uri.split('?').next() != Some(path)
        }),
        "unexpected DELETE {path}; requests: {requests:?}"
    );
}
