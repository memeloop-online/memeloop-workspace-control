pub(super) const SCHEMA_VERSION: i64 = 8;
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
