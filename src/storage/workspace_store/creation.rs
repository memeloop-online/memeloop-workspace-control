use sqlx::{PgConnection, SqliteConnection};
use uuid::Uuid;

use crate::{
    crypto::EnvelopeCipher,
    injections::InjectionItem,
    templates::WorkspaceTemplateDocument,
    workspaces::{Workspace, WorkspaceState},
};

use super::CreateWorkspace;
use crate::storage::{StorageError, WorkspaceInjectionRefs};

pub(super) struct WorkspaceCreation<'a> {
    pub(super) command: &'a CreateWorkspace,
    pub(super) injection_refs: &'a WorkspaceInjectionRefs,
    pub(super) inline: Option<(&'a EnvelopeCipher, &'a [InjectionItem])>,
    pub(super) admitted_template_yaml: Option<&'a str>,
    pub(super) allow_cluster_access: bool,
    pub(super) actor_user_id: Uuid,
    pub(super) now: i64,
}

pub(super) async fn create_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    creation: &WorkspaceCreation<'_>,
) -> Result<Workspace, StorageError> {
    let command = creation.command;
    let mut snapshot = crate::storage::template_store::resolve_template_sqlite(
        connection,
        installation_id,
        command.template_id,
        command.organization_id,
        creation.allow_cluster_access,
    )
    .await?;
    verify_admitted_template(creation, &snapshot)?;
    let yaml = apply_resource_override(command, &mut snapshot)?;
    let workspace = build_workspace(command, snapshot, creation.now);
    crate::storage::workspace_admission::admit_sqlite(connection, installation_id, &workspace)
        .await?;
    insert_sqlite(connection, installation_id, &workspace, &yaml, creation.now).await?;
    insert_injections_sqlite(connection, installation_id, &workspace, creation).await?;
    enqueue_and_audit_sqlite(connection, installation_id, &workspace, creation).await?;
    Ok(workspace)
}

pub(super) async fn create_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    creation: &WorkspaceCreation<'_>,
) -> Result<Workspace, StorageError> {
    let command = creation.command;
    let mut snapshot = crate::storage::template_store::resolve_template_postgres(
        connection,
        installation_id,
        command.template_id,
        command.organization_id,
        creation.allow_cluster_access,
    )
    .await?;
    verify_admitted_template(creation, &snapshot)?;
    let yaml = apply_resource_override(command, &mut snapshot)?;
    let workspace = build_workspace(command, snapshot, creation.now);
    crate::storage::workspace_admission::admit_postgres(connection, installation_id, &workspace)
        .await?;
    insert_postgres(connection, installation_id, &workspace, &yaml, creation.now).await?;
    insert_injections_postgres(connection, installation_id, &workspace, creation).await?;
    enqueue_and_audit_postgres(connection, installation_id, &workspace, creation).await?;
    Ok(workspace)
}

fn apply_resource_override(
    command: &CreateWorkspace,
    snapshot: &mut crate::storage::template_store::ResolvedTemplateSnapshot,
) -> Result<String, StorageError> {
    let Some(resources) = command.resources else {
        return Ok(snapshot.yaml.clone());
    };
    snapshot.spec.resources = resources;
    snapshot
        .spec
        .validate()
        .map_err(|_| StorageError::InvalidWorkspace)?;
    let mut document = WorkspaceTemplateDocument::parse(&snapshot.yaml)
        .map_err(|_| StorageError::InvalidTemplate)?;
    document.spec = snapshot.spec.clone();
    document
        .to_yaml()
        .map_err(|_| StorageError::InvalidWorkspace)
}

fn build_workspace(
    command: &CreateWorkspace,
    snapshot: crate::storage::template_store::ResolvedTemplateSnapshot,
    now: i64,
) -> Workspace {
    let id = Uuid::now_v7();
    // UUIDv7 starts with a timestamp, so its leading characters are identical for
    // workspaces created close together. Use the random tail for the human-facing
    // identifier to keep concurrent, bulk creation safe.
    Workspace {
        id,
        short_id: workspace_short_id(id),
        organization_id: command.organization_id,
        owner_id: command.owner_id,
        name: command.name.trim().to_owned(),
        template_id: Some(command.template_id),
        template: snapshot.spec,
        state: WorkspaceState::Provisioning,
        generation: 1,
        created_at: now,
        updated_at: now,
    }
}

fn workspace_short_id(id: Uuid) -> String {
    let encoded_id = id.simple().to_string();
    encoded_id[encoded_id.len() - 16..].to_owned()
}

fn verify_admitted_template(
    creation: &WorkspaceCreation<'_>,
    snapshot: &crate::storage::template_store::ResolvedTemplateSnapshot,
) -> Result<(), StorageError> {
    if creation
        .admitted_template_yaml
        .is_some_and(|expected| expected != snapshot.yaml)
    {
        return Err(StorageError::TemplateNotFound);
    }
    Ok(())
}

async fn insert_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace: &Workspace,
    yaml: &str,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO workspaces (id, installation_id, short_id, organization_id, owner_id, name, template_id, image, access_mode, state, cpu_millis, memory_mib, gpu_count, disk_gib, generation, created_at, updated_at, deleted_at, template_snapshot_yaml) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,1,?15,?15,NULL,?16)")
        .bind(workspace.id.to_string()).bind(installation_id).bind(&workspace.short_id).bind(workspace.organization_id.to_string()).bind(workspace.owner_id.to_string()).bind(&workspace.name).bind(workspace.template_id.map(|id| id.to_string())).bind(&workspace.template.image).bind(workspace.template.access_mode.as_str()).bind(workspace.state.as_str()).bind(as_i64(workspace.template.resources.cpu_millis)?).bind(as_i64(workspace.template.resources.memory_mib)?).bind(i64::from(workspace.template.resources.gpu_count)).bind(as_i64(workspace.template.resources.disk_gib)?).bind(now).bind(yaml).execute(&mut *connection).await?;
    Ok(())
}

async fn insert_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &Workspace,
    yaml: &str,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO workspaces (id, installation_id, short_id, organization_id, owner_id, name, template_id, image, access_mode, state, cpu_millis, memory_mib, gpu_count, disk_gib, generation, created_at, updated_at, deleted_at, template_snapshot_yaml) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,1,$15,$15,NULL,$16)")
        .bind(workspace.id.to_string()).bind(installation_id).bind(&workspace.short_id).bind(workspace.organization_id.to_string()).bind(workspace.owner_id.to_string()).bind(&workspace.name).bind(workspace.template_id.map(|id| id.to_string())).bind(&workspace.template.image).bind(workspace.template.access_mode.as_str()).bind(workspace.state.as_str()).bind(as_i64(workspace.template.resources.cpu_millis)?).bind(as_i64(workspace.template.resources.memory_mib)?).bind(i64::from(workspace.template.resources.gpu_count)).bind(as_i64(workspace.template.resources.disk_gib)?).bind(now).bind(yaml).execute(&mut *connection).await?;
    Ok(())
}

async fn insert_injections_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace: &Workspace,
    creation: &WorkspaceCreation<'_>,
) -> Result<(), StorageError> {
    crate::storage::workspace_injection_refs::insert_sqlite(
        connection,
        installation_id,
        workspace.id,
        creation.injection_refs,
        creation.now,
    )
    .await?;
    if let Some((cipher, items)) = creation.inline {
        for item in items {
            crate::storage::injection_store::insert_initial_workspace_injection_sqlite(
                connection,
                cipher,
                installation_id,
                workspace.id,
                item,
                creation.actor_user_id,
                creation.now,
            )
            .await?;
        }
    }
    Ok(())
}

async fn insert_injections_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &Workspace,
    creation: &WorkspaceCreation<'_>,
) -> Result<(), StorageError> {
    crate::storage::workspace_injection_refs::insert_postgres(
        connection,
        installation_id,
        workspace.id,
        creation.injection_refs,
        creation.now,
    )
    .await?;
    if let Some((cipher, items)) = creation.inline {
        for item in items {
            crate::storage::injection_store::insert_initial_workspace_injection_postgres(
                connection,
                cipher,
                installation_id,
                workspace.id,
                item,
                creation.actor_user_id,
                creation.now,
            )
            .await?;
        }
    }
    Ok(())
}

async fn enqueue_and_audit_sqlite(
    connection: &mut SqliteConnection,
    installation_id: &str,
    workspace: &Workspace,
    creation: &WorkspaceCreation<'_>,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES (?1,?2,'reconcile_workspace',?3,?4,'pending',?5,NULL,NULL,0,?5,?5)").bind(Uuid::now_v7().to_string()).bind(installation_id).bind(workspace.id.to_string()).bind(serde_json::json!({"generation": workspace.generation}).to_string()).bind(creation.now).execute(&mut *connection).await?;
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES (?1,?2,?3,?4,?5,'workspace.create',?6,?7)").bind(Uuid::now_v7().to_string()).bind(installation_id).bind(creation.actor_user_id.to_string()).bind(workspace.organization_id.to_string()).bind(workspace.id.to_string()).bind(serde_json::json!({"name": workspace.name, "image": workspace.template.image, "template_id": workspace.template_id, "resources": workspace.template.resources}).to_string()).bind(creation.now).execute(&mut *connection).await?;
    crate::storage::workspace_events::insert_sqlite(
        connection,
        installation_id,
        workspace,
        None,
        creation.now,
    )
    .await
}

async fn enqueue_and_audit_postgres(
    connection: &mut PgConnection,
    installation_id: &str,
    workspace: &Workspace,
    creation: &WorkspaceCreation<'_>,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO jobs (id, installation_id, kind, workspace_id, payload_json, status, available_at, lease_owner, lease_expires_at, attempts, created_at, updated_at) VALUES ($1,$2,'reconcile_workspace',$3,$4,'pending',$5,NULL,NULL,0,$5,$5)").bind(Uuid::now_v7().to_string()).bind(installation_id).bind(workspace.id.to_string()).bind(serde_json::json!({"generation": workspace.generation}).to_string()).bind(creation.now).execute(&mut *connection).await?;
    sqlx::query("INSERT INTO audit_log (id, installation_id, actor_user_id, organization_id, workspace_id, action, metadata_json, created_at) VALUES ($1,$2,$3,$4,$5,'workspace.create',$6,$7)").bind(Uuid::now_v7().to_string()).bind(installation_id).bind(creation.actor_user_id.to_string()).bind(workspace.organization_id.to_string()).bind(workspace.id.to_string()).bind(serde_json::json!({"name": workspace.name, "image": workspace.template.image, "template_id": workspace.template_id, "resources": workspace.template.resources}).to_string()).bind(creation.now).execute(&mut *connection).await?;
    crate::storage::workspace_events::insert_postgres(
        connection,
        installation_id,
        workspace,
        None,
        creation.now,
    )
    .await
}

fn as_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::InvalidWorkspace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_ids_use_the_uuid_random_tail_instead_of_the_shared_timestamp_prefix() {
        let first = Uuid::parse_str("01990f5d-6e80-7000-8000-000000000001").unwrap();
        let second = Uuid::parse_str("01990f5d-6e80-7000-8000-000000000002").unwrap();

        assert_eq!(workspace_short_id(first), "8000000000000001");
        assert_eq!(workspace_short_id(second), "8000000000000002");
    }
}
