pub(super) const SCHEMA_VERSION: i64 = 16;
pub(super) const MIGRATION_TABLE: &str = "CREATE TABLE IF NOT EXISTS schema_migrations (\
    version BIGINT PRIMARY KEY, applied_at BIGINT NOT NULL\
)";

pub(super) const MIGRATIONS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS installation_metadata (\
        singleton INTEGER PRIMARY KEY CHECK (singleton = 1), installation_id TEXT NOT NULL\
    )",
    "CREATE TABLE IF NOT EXISTS users (\
        id TEXT PRIMARY KEY, installation_id TEXT NOT NULL, display_name TEXT NOT NULL, \
        token_hash TEXT NOT NULL, system_admin BIGINT NOT NULL, disabled BIGINT NOT NULL, \
        created_at BIGINT NOT NULL, UNIQUE (installation_id, token_hash)\
    )",
    "CREATE INDEX IF NOT EXISTS users_installation_idx ON users (installation_id, id)",
    "CREATE TABLE IF NOT EXISTS organizations (\
        id TEXT PRIMARY KEY, installation_id TEXT NOT NULL, name TEXT NOT NULL, \
        created_at BIGINT NOT NULL, UNIQUE (installation_id, name)\
    )",
    "CREATE INDEX IF NOT EXISTS organizations_installation_idx \
        ON organizations (installation_id, id)",
    "CREATE TABLE IF NOT EXISTS organization_memberships (\
        installation_id TEXT NOT NULL, organization_id TEXT NOT NULL, user_id TEXT NOT NULL, \
        role TEXT NOT NULL, created_at BIGINT NOT NULL, \
        PRIMARY KEY (installation_id, organization_id, user_id), \
        FOREIGN KEY (organization_id) REFERENCES organizations(id) ON DELETE CASCADE, \
        FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE\
    )",
    "CREATE INDEX IF NOT EXISTS memberships_user_idx \
        ON organization_memberships (installation_id, user_id, organization_id)",
    "CREATE TABLE IF NOT EXISTS organization_quotas (\
        installation_id TEXT NOT NULL, organization_id TEXT NOT NULL, \
        cpu_millis BIGINT NOT NULL, memory_mib BIGINT NOT NULL, gpu_count BIGINT NOT NULL, \
        disk_gib BIGINT NOT NULL, updated_at BIGINT NOT NULL, \
        PRIMARY KEY (installation_id, organization_id), \
        FOREIGN KEY (organization_id) REFERENCES organizations(id) ON DELETE CASCADE\
    )",
    "CREATE TABLE IF NOT EXISTS user_quotas (\
        installation_id TEXT NOT NULL, user_id TEXT NOT NULL, \
        cpu_millis BIGINT NOT NULL, memory_mib BIGINT NOT NULL, gpu_count BIGINT NOT NULL, \
        disk_gib BIGINT NOT NULL, updated_at BIGINT NOT NULL, \
        PRIMARY KEY (installation_id, user_id), \
        FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE\
    )",
    "CREATE TABLE IF NOT EXISTS workspaces (\
        id TEXT PRIMARY KEY, installation_id TEXT NOT NULL, short_id TEXT NOT NULL, \
        organization_id TEXT NOT NULL, owner_id TEXT NOT NULL, name TEXT NOT NULL, \
        template_id TEXT, image TEXT NOT NULL, access_mode TEXT NOT NULL, state TEXT NOT NULL, \
        cpu_millis BIGINT NOT NULL, memory_mib BIGINT NOT NULL, gpu_count BIGINT NOT NULL, \
        disk_gib BIGINT NOT NULL, generation BIGINT NOT NULL, created_at BIGINT NOT NULL, \
        updated_at BIGINT NOT NULL, deleted_at BIGINT, \
        UNIQUE (installation_id, short_id), \
        UNIQUE (installation_id, organization_id, name), \
        FOREIGN KEY (organization_id) REFERENCES organizations(id), \
        FOREIGN KEY (owner_id) REFERENCES users(id)\
    )",
    "CREATE INDEX IF NOT EXISTS workspaces_org_idx \
        ON workspaces (installation_id, organization_id, state, created_at)",
    "CREATE TABLE IF NOT EXISTS workspace_injection_refs (\
        installation_id TEXT NOT NULL, workspace_id TEXT NOT NULL, scope TEXT NOT NULL, \
        injection_key TEXT NOT NULL, created_at BIGINT NOT NULL, \
        PRIMARY KEY (installation_id, workspace_id, scope, injection_key), \
        FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE\
    )",
    "CREATE TABLE IF NOT EXISTS audit_log (\
        id TEXT PRIMARY KEY, installation_id TEXT NOT NULL, actor_user_id TEXT, \
        organization_id TEXT, workspace_id TEXT, action TEXT NOT NULL, \
        metadata_json TEXT NOT NULL, created_at BIGINT NOT NULL\
    )",
    "CREATE INDEX IF NOT EXISTS audit_installation_idx \
        ON audit_log (installation_id, created_at, id)",
    "CREATE TABLE IF NOT EXISTS injection_items (\
        id TEXT PRIMARY KEY, installation_id TEXT NOT NULL, scope TEXT NOT NULL, \
        scope_id TEXT NOT NULL, key TEXT NOT NULL, kind TEXT NOT NULL, target TEXT NOT NULL, \
        value_encoding TEXT NOT NULL, ciphertext TEXT NOT NULL, value_nonce TEXT NOT NULL, \
        wrapped_data_key TEXT NOT NULL, key_nonce TEXT NOT NULL, sensitive BIGINT NOT NULL, \
        locked BIGINT NOT NULL, version BIGINT NOT NULL, file_mode BIGINT, owner_name TEXT, \
        group_name TEXT, template_selector TEXT, labels_json TEXT NOT NULL, \
        created_by TEXT NOT NULL, created_at BIGINT NOT NULL, updated_at BIGINT NOT NULL, \
        UNIQUE (installation_id, scope, scope_id, key)\
    )",
    "CREATE INDEX IF NOT EXISTS injections_scope_idx \
        ON injection_items (installation_id, scope, scope_id, key)",
    "CREATE TABLE IF NOT EXISTS jobs (\
        id TEXT PRIMARY KEY, installation_id TEXT NOT NULL, kind TEXT NOT NULL, \
        workspace_id TEXT, payload_json TEXT NOT NULL, status TEXT NOT NULL, \
        available_at BIGINT NOT NULL, lease_owner TEXT, lease_expires_at BIGINT, \
        attempts BIGINT NOT NULL, created_at BIGINT NOT NULL, updated_at BIGINT NOT NULL\
    )",
    "CREATE INDEX IF NOT EXISTS jobs_claim_idx \
        ON jobs (installation_id, status, available_at, lease_expires_at)",
    "CREATE TABLE IF NOT EXISTS workspace_leases (\
        installation_id TEXT NOT NULL, workspace_id TEXT NOT NULL, lease_owner TEXT NOT NULL, \
        lease_expires_at BIGINT NOT NULL, updated_at BIGINT NOT NULL, \
        PRIMARY KEY (installation_id, workspace_id)\
    )",
    "CREATE TABLE IF NOT EXISTS events (\
        id TEXT PRIMARY KEY, installation_id TEXT NOT NULL, organization_id TEXT NOT NULL, \
        workspace_id TEXT, kind TEXT NOT NULL, payload_json TEXT NOT NULL, created_at BIGINT NOT NULL\
    )",
    "CREATE INDEX IF NOT EXISTS events_installation_idx \
        ON events (installation_id, created_at, id)",
    "CREATE TABLE IF NOT EXISTS web_shell_tickets (\
        id TEXT PRIMARY KEY, installation_id TEXT NOT NULL, ticket_hash TEXT NOT NULL, \
        organization_id TEXT NOT NULL, workspace_id TEXT NOT NULL, user_id TEXT NOT NULL, \
        expires_at BIGINT NOT NULL, consumed_at BIGINT, created_at BIGINT NOT NULL, \
        UNIQUE (installation_id, ticket_hash)\
    )",
    "CREATE INDEX IF NOT EXISTS web_shell_tickets_expiry_idx \
        ON web_shell_tickets (installation_id, expires_at, consumed_at)",
    "CREATE TABLE IF NOT EXISTS idempotency_keys (\
        installation_id TEXT NOT NULL, scope TEXT NOT NULL, key TEXT NOT NULL, \
        request_hash TEXT NOT NULL, response_json TEXT NOT NULL, status_code BIGINT NOT NULL, \
        created_at BIGINT NOT NULL, expires_at BIGINT NOT NULL, \
        PRIMARY KEY (installation_id, scope, key)\
    )",
    "CREATE TABLE IF NOT EXISTS image_policies (\
        installation_id TEXT NOT NULL, image TEXT NOT NULL, contract_version BIGINT NOT NULL, \
        enabled BIGINT NOT NULL, created_at BIGINT NOT NULL, updated_at BIGINT NOT NULL, \
        PRIMARY KEY (installation_id, image)\
    )",
    "CREATE TABLE IF NOT EXISTS workspace_templates (\
        id TEXT PRIMARY KEY, installation_id TEXT NOT NULL, organization_id TEXT, \
        name TEXT NOT NULL, image TEXT NOT NULL, access_mode TEXT NOT NULL, \
        cpu_millis BIGINT NOT NULL, memory_mib BIGINT NOT NULL, gpu_count BIGINT NOT NULL, \
        disk_gib BIGINT NOT NULL, enabled BIGINT NOT NULL, created_at BIGINT NOT NULL, \
        updated_at BIGINT NOT NULL, UNIQUE (installation_id, organization_id, name)\
    )",
    "CREATE INDEX IF NOT EXISTS templates_installation_idx ON workspace_templates \
        (installation_id, organization_id, enabled, name)",
    "CREATE TABLE IF NOT EXISTS webhook_subscriptions (\
        id TEXT PRIMARY KEY, installation_id TEXT NOT NULL, organization_id TEXT NOT NULL, \
        url TEXT NOT NULL, event_prefix TEXT NOT NULL, ciphertext TEXT NOT NULL, \
        value_nonce TEXT NOT NULL, wrapped_data_key TEXT NOT NULL, key_nonce TEXT NOT NULL, \
        enabled BIGINT NOT NULL, created_by TEXT NOT NULL, created_at BIGINT NOT NULL, \
        updated_at BIGINT NOT NULL\
    )",
    "CREATE INDEX IF NOT EXISTS webhooks_org_idx ON webhook_subscriptions \
        (installation_id, organization_id, enabled, id)",
    "CREATE TABLE IF NOT EXISTS workspace_ssh_identities (\
        installation_id TEXT NOT NULL, workspace_id TEXT NOT NULL, public_key TEXT NOT NULL, \
        fingerprint TEXT NOT NULL, ciphertext TEXT NOT NULL, value_nonce TEXT NOT NULL, \
        wrapped_data_key TEXT NOT NULL, key_nonce TEXT NOT NULL, created_at BIGINT NOT NULL, \
        PRIMARY KEY (installation_id, workspace_id)\
    )",
    "CREATE TABLE IF NOT EXISTS workspace_tombstones (\
        installation_id TEXT NOT NULL, workspace_id TEXT NOT NULL, organization_id TEXT NOT NULL, \
        deleted_at BIGINT NOT NULL, PRIMARY KEY (installation_id, workspace_id)\
    )",
    "ALTER TABLE workspace_templates ADD COLUMN runtime_profile TEXT NOT NULL DEFAULT 'standard'",
    "ALTER TABLE workspaces ADD COLUMN runtime_profile TEXT NOT NULL DEFAULT 'standard'",
];

pub(super) const PROFILE_RENAME_MIGRATIONS: &[&str] = &[
    "UPDATE workspace_templates SET runtime_profile = CASE runtime_profile \
        WHEN 'coder_rust_dev' THEN 'rust_dev' \
        WHEN 'coder_token_center_rust_dev' THEN 'rust_dev' \
        WHEN 'coder_node_dev' THEN 'node_dev' \
        WHEN 'coder_cluster_admin' THEN 'maintainance' \
        ELSE runtime_profile END",
    "UPDATE workspaces SET runtime_profile = CASE runtime_profile \
        WHEN 'coder_rust_dev' THEN 'rust_dev' \
        WHEN 'coder_token_center_rust_dev' THEN 'rust_dev' \
        WHEN 'coder_node_dev' THEN 'node_dev' \
        WHEN 'coder_cluster_admin' THEN 'maintainance' \
        ELSE runtime_profile END",
];

pub(super) const TEMPLATE_YAML_MIGRATIONS: &[&str] = &[
    "ALTER TABLE workspace_templates ADD COLUMN template_yaml TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE workspaces ADD COLUMN template_snapshot_yaml TEXT NOT NULL DEFAULT ''",
];

pub(super) const PLUGIN_CONFIGURATION_MIGRATIONS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS plugin_configurations (\
        installation_id TEXT NOT NULL, plugin_id TEXT NOT NULL, scope_key TEXT NOT NULL, \
        scope_kind TEXT NOT NULL CHECK (scope_kind IN ('installation', 'organization')), \
        organization_id TEXT, value_json TEXT NOT NULL, schema_digest TEXT NOT NULL, \
        version BIGINT NOT NULL, \
        updated_by TEXT NOT NULL, updated_at BIGINT NOT NULL, \
        PRIMARY KEY (installation_id, plugin_id, scope_key), \
        FOREIGN KEY (organization_id) REFERENCES organizations(id) ON DELETE CASCADE, \
        FOREIGN KEY (updated_by) REFERENCES users(id)\
    )",
    "CREATE INDEX IF NOT EXISTS plugin_configurations_organization_idx ON plugin_configurations \
        (installation_id, organization_id, plugin_id)",
];

pub(super) const DYNAMIC_PLUGIN_MIGRATIONS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS plugin_packages (\
        installation_id TEXT NOT NULL, plugin_id TEXT NOT NULL, manifest_json TEXT NOT NULL, \
        component_bytes BYTEA, package_digest TEXT NOT NULL, source_kind TEXT NOT NULL, \
        source_ref TEXT NOT NULL, source_confirmation TEXT NOT NULL, enabled BIGINT NOT NULL, \
        approved_contributions_json TEXT NOT NULL, version BIGINT NOT NULL, \
        created_by TEXT NOT NULL, created_at BIGINT NOT NULL, updated_at BIGINT NOT NULL, \
        PRIMARY KEY (installation_id, plugin_id), FOREIGN KEY (created_by) REFERENCES users(id)\
    )",
    "CREATE TABLE IF NOT EXISTS plugin_install_inspections (\
        id TEXT PRIMARY KEY, installation_id TEXT NOT NULL, plugin_id TEXT NOT NULL, \
        manifest_json TEXT NOT NULL, component_bytes BYTEA, package_digest TEXT NOT NULL, \
        size_bytes BIGINT NOT NULL, source_kind TEXT NOT NULL, source_ref TEXT NOT NULL, \
        source_confirmation TEXT NOT NULL, declared_contributions_json TEXT NOT NULL, \
        assets_json TEXT NOT NULL, \
        created_by TEXT NOT NULL, created_at BIGINT NOT NULL, expires_at BIGINT NOT NULL, \
        FOREIGN KEY (created_by) REFERENCES users(id)\
    )",
    "CREATE INDEX IF NOT EXISTS plugin_inspections_expiry_idx ON plugin_install_inspections \
        (installation_id, expires_at)",
    "CREATE TABLE IF NOT EXISTS plugin_assets (\
        installation_id TEXT NOT NULL, plugin_id TEXT NOT NULL, asset_path TEXT NOT NULL, \
        media_type TEXT NOT NULL, content_bytes BYTEA NOT NULL, content_digest TEXT NOT NULL, \
        PRIMARY KEY (installation_id, plugin_id, asset_path), \
        FOREIGN KEY (installation_id, plugin_id) REFERENCES plugin_packages(installation_id, plugin_id) \
            ON DELETE CASCADE\
    )",
    "CREATE TABLE IF NOT EXISTS plugin_ui_sessions (\
        id TEXT PRIMARY KEY, installation_id TEXT NOT NULL, plugin_id TEXT NOT NULL, \
        surface_id TEXT NOT NULL, user_id TEXT NOT NULL, ticket_hash TEXT NOT NULL, \
        cookie_hash TEXT NOT NULL, channel_nonce TEXT NOT NULL, \
        allowed_bridge_methods_json TEXT NOT NULL, entrypoint TEXT NOT NULL, package_digest TEXT NOT NULL, \
        expires_at BIGINT NOT NULL, consumed_at BIGINT, created_at BIGINT NOT NULL, \
        UNIQUE (installation_id, ticket_hash), FOREIGN KEY (user_id) REFERENCES users(id), \
        FOREIGN KEY (installation_id, plugin_id) REFERENCES plugin_packages(installation_id, plugin_id) \
            ON DELETE CASCADE\
    )",
    "CREATE TABLE IF NOT EXISTS plugin_catalog_metadata (\
        installation_id TEXT PRIMARY KEY, revision BIGINT NOT NULL\
    )",
];

pub(super) const USER_SETTINGS_MIGRATIONS: &[&str] = &[
    "ALTER TABLE users ADD COLUMN avatar_url TEXT",
    "CREATE TABLE IF NOT EXISTS user_api_keys (\
        id TEXT PRIMARY KEY, installation_id TEXT NOT NULL, user_id TEXT NOT NULL, \
        name TEXT NOT NULL, token_prefix TEXT NOT NULL, token_hash TEXT NOT NULL, \
        last_used_at BIGINT, created_at BIGINT NOT NULL, revoked_at BIGINT, \
        UNIQUE (installation_id, token_hash), \
        FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE\
    )",
    "CREATE INDEX IF NOT EXISTS user_api_keys_user_idx ON user_api_keys \
        (installation_id, user_id, revoked_at, created_at, id)",
    "INSERT INTO user_api_keys (id, installation_id, user_id, name, token_prefix, token_hash, \
        last_used_at, created_at, revoked_at) \
        SELECT id, installation_id, id, 'Legacy key', 'legacy', token_hash, NULL, created_at, NULL \
        FROM users WHERE true ON CONFLICT (installation_id, token_hash) DO NOTHING",
];

pub(super) const V15_MIGRATIONS: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS users_page_idx ON users \
        (installation_id, created_at, id)",
    "CREATE INDEX IF NOT EXISTS organizations_page_idx ON organizations \
        (installation_id, created_at, id)",
    "CREATE INDEX IF NOT EXISTS workspaces_page_idx ON workspaces \
        (installation_id, organization_id, state, created_at, id)",
    "CREATE TABLE IF NOT EXISTS workspace_port_mappings (\
        id TEXT PRIMARY KEY, installation_id TEXT NOT NULL, organization_id TEXT NOT NULL, \
        workspace_id TEXT NOT NULL, internal_port BIGINT NOT NULL CHECK (internal_port BETWEEN 1 AND 65535), \
        display_name TEXT, created_by TEXT NOT NULL, created_at BIGINT NOT NULL, \
        UNIQUE (installation_id, workspace_id, internal_port), \
        FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE, \
        FOREIGN KEY (created_by) REFERENCES users(id)\
    )",
    "CREATE INDEX IF NOT EXISTS workspace_port_mappings_workspace_idx ON workspace_port_mappings \
        (installation_id, workspace_id, created_at, id)",
    "CREATE TABLE IF NOT EXISTS workspace_port_mapping_tickets (\
        id TEXT PRIMARY KEY, installation_id TEXT NOT NULL, mapping_id TEXT NOT NULL, \
        user_id TEXT NOT NULL, ticket_hash TEXT NOT NULL, expires_at BIGINT NOT NULL, \
        consumed_at BIGINT, created_at BIGINT NOT NULL, UNIQUE (installation_id, ticket_hash), \
        FOREIGN KEY (mapping_id) REFERENCES workspace_port_mappings(id) ON DELETE CASCADE, \
        FOREIGN KEY (user_id) REFERENCES users(id)\
    )",
    "CREATE INDEX IF NOT EXISTS workspace_port_mapping_tickets_expiry_idx \
        ON workspace_port_mapping_tickets (installation_id, expires_at, consumed_at)",
    "CREATE TABLE IF NOT EXISTS workspace_port_mapping_sessions (\
        id TEXT PRIMARY KEY, installation_id TEXT NOT NULL, mapping_id TEXT NOT NULL, \
        user_id TEXT NOT NULL, session_hash TEXT NOT NULL, expires_at BIGINT NOT NULL, \
        revoked_at BIGINT, created_at BIGINT NOT NULL, UNIQUE (installation_id, session_hash), \
        FOREIGN KEY (mapping_id) REFERENCES workspace_port_mappings(id) ON DELETE CASCADE, \
        FOREIGN KEY (user_id) REFERENCES users(id)\
    )",
    "CREATE INDEX IF NOT EXISTS workspace_port_mapping_sessions_expiry_idx \
        ON workspace_port_mapping_sessions (installation_id, expires_at, revoked_at)",
];

/// Indexes added after the initial pagination schema. Keep this group separate so an
/// installation already at v15 receives the index without replaying older DDL.
pub(super) const V16_MIGRATIONS: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS memberships_organization_role_idx \
        ON organization_memberships (installation_id, organization_id, role)",
];
