/// Schema v15.  Kept separate from the schema registry so migration ordering remains owned by
/// `storage::schema`; wire this slice in as the next migration group.
pub(in crate::storage) const API_KEY_SCOPE_MIGRATIONS: &[&str] = &[
    "ALTER TABLE user_api_keys ADD COLUMN scopes_json TEXT NOT NULL DEFAULT '[\"*\"]'",
    "ALTER TABLE user_api_keys ADD COLUMN expires_at BIGINT",
    "CREATE INDEX IF NOT EXISTS user_api_keys_auth_idx ON user_api_keys (installation_id, token_hash, revoked_at, expires_at)",
];
