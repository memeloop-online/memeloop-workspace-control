use super::*;
use memeloop_workspace_control::{
    injections::{InjectionItem, InjectionScope},
    quota::Resources,
    storage::{CreateOrganization, CreateWorkspaceTemplate, InjectionScopeRef, WorkspaceTemplate},
    templates::{WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::AccessMode,
};

async fn fixture() -> (Router, Database, Uuid, Uuid, [WorkspaceTemplate; 2]) {
    let (app, database, user) = app(true).await;
    let organization = database
        .create_organization(
            CreateOrganization {
                name: "Preview selectors".to_owned(),
                owner_user_id: user,
            },
            2,
        )
        .await
        .unwrap();
    let mut templates = Vec::new();
    for name in ["A", "B"] {
        let template = database
            .create_workspace_template(
                CreateWorkspaceTemplate {
                    organization_id: Some(organization.id),
                    yaml: WorkspaceTemplateDocument::new(
                        name,
                        WorkspaceTemplateSpec::standard(
                            format!("registry.example/workspace:{name}"),
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
        templates.push(template);
    }
    let cipher = EnvelopeCipher::from_base64(&STANDARD.encode([9_u8; 32])).unwrap();
    let mut items = Vec::new();
    for (index, template) in templates.iter().enumerate() {
        items.push((
            InjectionScope::Organization,
            organization.id,
            json!({
                "key": format!("docker-{index}"), "kind": "environment_variable", "target": "DOCKER_CONFIG",
                "value": {"encoding": "utf8", "value": "private-docker-path"},
                "sensitive": true, "locked": true, "version": 0,
                "template_selector": template.id.to_string(), "labels": {}
            }),
        ));
        items.push((
            InjectionScope::User,
            user,
            json!({
                "key": format!("image-{index}"), "kind": "environment_variable", "target": "IMAGE_CONFIG",
                "value": {"encoding": "utf8", "value": "private-image-path"},
                "sensitive": true, "locked": false, "version": 0,
                "labels": {"image": template.template.image, "access_mode": "internal"}
            }),
        ));
    }
    items.push((
        InjectionScope::User,
        user,
        json!({
            "key": "shell-skill", "kind": "config_file", "target": "/home/user/.codex/skills/proxy-imagegen/SKILL.md",
            "value": {"encoding": "utf8", "value": "private-skill-content"},
            "sensitive": false, "locked": false, "version": 0, "file_mode": 420,
            "owner": "user", "group": "user", "labels": {}
        }),
    ));
    for (scope, scope_id, value) in items {
        let item: InjectionItem = serde_json::from_value(value).unwrap();
        database
            .replace_injection(
                &cipher,
                InjectionScopeRef { scope, scope_id },
                item,
                user,
                4,
            )
            .await
            .unwrap();
    }
    (
        app,
        database,
        user,
        organization.id,
        templates.try_into().unwrap(),
    )
}

#[tokio::test]
async fn creation_preview_filters_template_and_image_before_resolving_targets_or_refs() {
    let (app, _, user, organization, templates) = fixture().await;
    for (index, template) in templates.iter().enumerate() {
        let response = app
            .clone()
            .oneshot(request(
                Method::POST,
                "/api/v1/injections/preview",
                None,
                Some(json!({
                    "organization_id": organization, "user_id": user, "template_id": template.id
                })),
            ))
            .await
            .unwrap();
        let (status, body) = response_body(response).await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let rows: Vec<Value> = serde_json::from_slice(&body).unwrap();
        let keys = rows
            .iter()
            .map(|row| row["key"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        let docker = format!("docker-{index}");
        let image = format!("image-{index}");
        assert_eq!(
            keys,
            [docker.as_str(), image.as_str(), "shell-skill"]
                .into_iter()
                .collect()
        );
        assert!(!String::from_utf8_lossy(&body).contains("private-"));
        let locked = app
            .clone()
            .oneshot(request(
                Method::POST,
                "/api/v1/injections/preview",
                None,
                Some(json!({
                    "organization_id": organization, "user_id": user, "template_id": template.id,
                    "organization_injection_refs": [], "user_injection_refs": []
                })),
            ))
            .await
            .unwrap();
        let (status, body) = response_body(locked).await;
        assert_eq!(status, StatusCode::OK);
        let rows: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["key"], docker);
        let invalid = app
            .clone()
            .oneshot(request(
                Method::POST,
                "/api/v1/injections/preview",
                None,
                Some(json!({
                    "organization_id": organization, "user_id": user, "template_id": template.id,
                    "organization_injection_refs": [format!("docker-{}", 1-index)]
                })),
            ))
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    }
    let missing = app
        .oneshot(request(
            Method::POST,
            "/api/v1/injections/preview",
            None,
            Some(json!({
                "organization_id": organization, "user_id": user
            })),
        ))
        .await
        .unwrap();
    let (status, body) = response_body(missing).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(String::from_utf8_lossy(&body).contains("template_id is required"));
}

#[tokio::test]
async fn creation_preview_rejects_disabled_foreign_and_token_restricted_templates() {
    let (app, database, user, organization, templates) = fixture().await;
    let other = database
        .create_organization(
            CreateOrganization {
                name: "Other organization".to_owned(),
                owner_user_id: user,
            },
            5,
        )
        .await
        .unwrap();
    let foreign = app
        .clone()
        .oneshot(request(
            Method::POST,
            "/api/v1/injections/preview",
            None,
            Some(json!({
                "organization_id": other.id, "user_id": user, "template_id": templates[0].id
            })),
        ))
        .await
        .unwrap();
    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
    database
        .set_workspace_template_enabled(templates[1].id, false, 6)
        .await
        .unwrap();
    let disabled = app
        .clone()
        .oneshot(request(
            Method::POST,
            "/api/v1/injections/preview",
            None,
            Some(json!({
                "organization_id": organization, "user_id": user, "template_id": templates[1].id
            })),
        ))
        .await
        .unwrap();
    assert_eq!(disabled.status(), StatusCode::NOT_FOUND);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let restricted = database
        .create_api_key(
            user,
            "Preview A only",
            vec![ApiKeyScope::ReadWorkspace],
            Some(now + 3600),
            Some(vec![templates[0].id]),
            now,
        )
        .await
        .unwrap();
    for (index, status) in [(0, StatusCode::OK), (1, StatusCode::FORBIDDEN)] {
        let response = app.clone().oneshot(request_as(&restricted.token, Method::POST, "/api/v1/injections/preview", None, Some(json!({
            "organization_id": organization, "user_id": user, "template_id": templates[index].id
        })))).await.unwrap();
        assert_eq!(response.status(), status);
    }
}
