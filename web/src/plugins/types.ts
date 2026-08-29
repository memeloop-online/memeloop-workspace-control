export interface PluginManifest {
  id: string;
  name: string;
  version: string;
  description: string;
  wit_version: string;
  workspace_create_policy: boolean;
  denial_codes: string[];
  declared_contributions: string[];
  approved_contributions: string[];
  configuration_schema?: Record<string, unknown> | null;
  configuration_default?: unknown;
  loaded: boolean;
  source?: string;
  error_code?: string | null;
  error_message?: string | null;
}

export type PluginConfigurationScope = "installation" | "organization";
export type PluginConfigurationSource = "default" | PluginConfigurationScope;

export interface PluginConfiguration {
  plugin_id: string;
  scope: PluginConfigurationScope;
  organization_id: string | null;
  source: PluginConfigurationSource;
  scope_version: number;
  effective_version: number;
  value: unknown;
  schema_digest: string;
  stored_schema_digest: string | null;
  schema_changed: boolean;
  valid: boolean;
  updated_at: number | null;
}

export interface PutPluginConfiguration {
  expected_version: number;
  value: unknown;
}
