//! Compatibility code for records written before template YAML became authoritative.
//!
//! Keep historical Coder names and deployment-specific defaults out of the current
//! template domain. This module is only used by database migration and snapshot import.

use std::collections::BTreeMap;

use sqlx::Row;

use crate::{
    quota::Resources,
    templates::{PodResourceRequest, WorkspaceTemplateDocument, WorkspaceTemplateSpec},
    workspaces::AccessMode,
};

use super::{Database, StorageError};

const NO_PROXY_NODE: &str = "localhost,127.0.0.1,::1,.svc,.cluster.local,10.42.0.0/16,10.43.0.0/16,100.64.0.0/10,.k3s.onetwo.website,npmmirror.com";
const NO_PROXY_RUST: &str = "localhost,127.0.0.1,::1,.svc,.cluster.local,10.42.0.0/16,10.43.0.0/16,100.64.0.0/10,.k3s.onetwo.website,rsproxy.cn,npmmirror.com";

pub(super) fn from_legacy(
    legacy_value: &str,
    image: impl Into<String>,
    access_mode: AccessMode,
    resources: Resources,
) -> Result<WorkspaceTemplateSpec, StorageError> {
    let mut spec = WorkspaceTemplateSpec::standard(image, access_mode, resources);
    match legacy_value {
        "standard" => {}
        "node_dev" | "coder_node_dev" => configure_node(&mut spec, resources),
        "rust_dev" | "coder_rust_dev" | "coder_token_center_rust_dev" => {
            configure_rust(&mut spec, resources)
        }
        "maintainance" | "coder_cluster_admin" => configure_maintenance(&mut spec, resources),
        _ => return Err(StorageError::InvalidTemplate),
    }
    spec.validate().map_err(|_| StorageError::InvalidTemplate)?;
    Ok(spec)
}

pub(super) async fn backfill(database: &Database) -> Result<(), StorageError> {
    match database {
        Database::Sqlite {
            pool,
            installation_id,
        } => {
            for table in ["workspace_templates", "workspaces"] {
                let yaml_column = yaml_column(table);
                let query = legacy_rows_query(table, yaml_column, "?1");
                for row in sqlx::query(&query)
                    .bind(installation_id.as_str())
                    .fetch_all(pool)
                    .await?
                {
                    let (id, yaml) = legacy_yaml(&row)?;
                    let update = backfill_query(table, yaml_column, "?1", "?2", "?3");
                    sqlx::query(&update)
                        .bind(yaml)
                        .bind(installation_id.as_str())
                        .bind(id)
                        .execute(pool)
                        .await?;
                }
            }
        }
        Database::Postgres {
            pool,
            installation_id,
        } => {
            for table in ["workspace_templates", "workspaces"] {
                let yaml_column = yaml_column(table);
                let query = legacy_rows_query(table, yaml_column, "$1");
                for row in sqlx::query(&query)
                    .bind(installation_id.as_str())
                    .fetch_all(pool)
                    .await?
                {
                    let (id, yaml) = legacy_yaml(&row)?;
                    let update = backfill_query(table, yaml_column, "$1", "$2", "$3");
                    sqlx::query(&update)
                        .bind(yaml)
                        .bind(installation_id.as_str())
                        .bind(id)
                        .execute(pool)
                        .await?;
                }
            }
        }
    }
    Ok(())
}

fn yaml_column(table: &str) -> &'static str {
    match table {
        "workspace_templates" => "template_yaml",
        "workspaces" => "template_snapshot_yaml",
        _ => unreachable!("migration table is selected internally"),
    }
}

fn legacy_rows_query(table: &str, yaml_column: &str, installation: &str) -> String {
    format!(
        "SELECT id, name, runtime_profile, image, access_mode, cpu_millis, memory_mib, gpu_count, disk_gib FROM {table} WHERE installation_id = {installation} AND {yaml_column} = ''"
    )
}

fn backfill_query(
    table: &str,
    yaml_column: &str,
    yaml: &str,
    installation: &str,
    id: &str,
) -> String {
    format!(
        "UPDATE {table} SET {yaml_column} = {yaml} WHERE installation_id = {installation} AND id = {id} AND {yaml_column} = ''"
    )
}

pub(super) fn legacy_yaml<R: Row>(row: &R) -> Result<(String, String), StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
    i64: for<'d> sqlx::Decode<'d, R::Database> + sqlx::Type<R::Database>,
{
    let access = row.try_get::<String, _>("access_mode")?;
    let resources = Resources {
        cpu_millis: as_u64(row.try_get("cpu_millis")?)?,
        memory_mib: as_u64(row.try_get("memory_mib")?)?,
        gpu_count: u32::try_from(row.try_get::<i64, _>("gpu_count")?)
            .map_err(|_| StorageError::InvalidTemplate)?,
        disk_gib: as_u64(row.try_get("disk_gib")?)?,
    };
    let spec = from_legacy(
        &row.try_get::<String, _>("runtime_profile")?,
        row.try_get::<String, _>("image")?,
        AccessMode::from_database(&access).ok_or(StorageError::UnknownAccessMode(access))?,
        resources,
    )?;
    let document = WorkspaceTemplateDocument::new(row.try_get::<String, _>("name")?, spec);
    Ok((
        row.try_get("id")?,
        document
            .to_yaml()
            .map_err(|_| StorageError::InvalidTemplate)?,
    ))
}

fn configure_node(spec: &mut WorkspaceTemplateSpec, resources: Resources) {
    spec.workspace_user = "node-dev".to_owned();
    spec.workspace_home = "/home/node-dev".to_owned();
    spec.preserve_home_ownership = true;
    spec.buildkit = true;
    spec.pod_requests = PodResourceRequest {
        cpu_millis: 1_000.min(resources.cpu_millis),
        memory_mib: 1_024.min(resources.memory_mib),
        ephemeral_storage_mib: Some(256),
    };
    spec.ephemeral_storage_limit_mib = Some(1_024);
    spec.required_node_names = vec!["westlake".to_owned(), "haixia".to_owned()];
    spec.environment = development_environment(&spec.workspace_home, NO_PROXY_NODE, false);
}

fn configure_rust(spec: &mut WorkspaceTemplateSpec, resources: Resources) {
    spec.workspace_user = "rust-dev".to_owned();
    spec.workspace_home = "/home/rust-dev".to_owned();
    spec.preserve_home_ownership = true;
    spec.buildkit = true;
    spec.pod_requests = PodResourceRequest {
        cpu_millis: 2_000.min(resources.cpu_millis),
        memory_mib: 4_096.min(resources.memory_mib),
        ephemeral_storage_mib: Some(256),
    };
    spec.ephemeral_storage_limit_mib = Some(1_024);
    spec.required_node_names = vec!["westlake".to_owned(), "haixia".to_owned()];
    spec.environment = development_environment(&spec.workspace_home, NO_PROXY_RUST, true);
    spec.environment
        .insert("RUSTUP_HOME".to_owned(), "/usr/local/rustup".to_owned());
    spec.environment.insert(
        "RUSTUP_DIST_SERVER".to_owned(),
        "https://rsproxy.cn".to_owned(),
    );
    spec.environment.insert(
        "RUSTUP_UPDATE_ROOT".to_owned(),
        "https://rsproxy.cn/rustup".to_owned(),
    );
}

fn configure_maintenance(spec: &mut WorkspaceTemplateSpec, resources: Resources) {
    spec.workspace_user = "cluster-admin".to_owned();
    spec.workspace_home = "/home/cluster-admin".to_owned();
    spec.preserve_home_ownership = true;
    spec.cluster_access = true;
    spec.pod_requests.cpu_millis = 100.min(resources.cpu_millis);
    spec.preferred_node_names = vec!["westlake".to_owned()];
    spec.node_selector
        .insert("k3s-worker-ready".to_owned(), "true".to_owned());
    spec.environment
        .insert("HOME".to_owned(), spec.workspace_home.clone());
    spec.environment.insert(
        "KUBECONFIG".to_owned(),
        format!("{}/.mwc/kubeconfig", spec.workspace_home),
    );
    spec.environment.insert("PATH".to_owned(), format!("/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin:{}/.local/bin:{}/.local/share/pnpm", spec.workspace_home, spec.workspace_home));
}

fn development_environment(home: &str, no_proxy: &str, rust: bool) -> BTreeMap<String, String> {
    let temporary_directory = if rust {
        "/tmp".to_owned()
    } else {
        format!("{home}/.tmp/dev")
    };
    let mut values = BTreeMap::from([
        ("HOME".to_owned(), home.to_owned()),
        ("NO_PROXY".to_owned(), no_proxy.to_owned()),
        ("no_proxy".to_owned(), no_proxy.to_owned()),
        ("TMPDIR".to_owned(), temporary_directory.clone()),
        ("TMP".to_owned(), temporary_directory.clone()),
        ("TEMP".to_owned(), temporary_directory),
        ("XDG_CACHE_HOME".to_owned(), format!("{home}/.cache")),
        ("XDG_CONFIG_HOME".to_owned(), format!("{home}/.config")),
        ("XDG_DATA_HOME".to_owned(), format!("{home}/.local/share")),
        ("XDG_STATE_HOME".to_owned(), format!("{home}/.local/state")),
        (
            "PLAYWRIGHT_BROWSERS_PATH".to_owned(),
            format!("{home}/.cache/ms-playwright"),
        ),
        ("DOCKER_CONFIG".to_owned(), format!("{home}/.config/docker")),
        ("NPM_CONFIG_CACHE".to_owned(), format!("{home}/.cache/npm")),
        ("PNPM_HOME".to_owned(), format!("{home}/.local/share/pnpm")),
        (
            "YARN_CACHE_FOLDER".to_owned(),
            format!("{home}/.cache/yarn"),
        ),
        (
            "BUILDKIT_HOST".to_owned(),
            format!("unix://{home}/.cache/buildkit/runtime/buildkit/buildkitd.sock"),
        ),
    ]);
    let path = if rust {
        format!(
            "/usr/local/bin:/usr/local/sbin:/usr/local/cargo/bin:{home}/.local/bin:{home}/.local/share/pnpm:{home}/.cargo/bin:/usr/sbin:/usr/bin:/sbin:/bin"
        )
    } else {
        format!(
            "/usr/local/bin:/usr/local/sbin:{home}/.local/bin:{home}/.local/share/pnpm:/usr/sbin:/usr/bin:/sbin:/bin"
        )
    };
    values.insert("PATH".to_owned(), path);
    if rust {
        values.extend([
            ("CARGO_HOME".to_owned(), format!("{home}/.cargo")),
            (
                "CARGO_TARGET_DIR".to_owned(),
                format!("{home}/.cache/cargo-target"),
            ),
            ("CARGO_HTTP_MULTIPLEXING".to_owned(), "false".to_owned()),
            (
                "CARGO_REGISTRIES_CRATES_IO_PROTOCOL".to_owned(),
                "sparse".to_owned(),
            ),
        ]);
    }
    values
}

fn as_u64(value: i64) -> Result<u64, StorageError> {
    u64::try_from(value).map_err(|_| StorageError::InvalidTemplate)
}
