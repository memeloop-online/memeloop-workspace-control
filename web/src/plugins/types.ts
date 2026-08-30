export interface PluginManifest {
  id: string;
  name: string;
  version: string;
  description: string;
  wit_version: string;
  workspace_create_policy: boolean;
  denial_codes: string[];
  declared_contributions: PluginContribution[];
  approved_contributions: PluginContribution[];
  configuration_schema?: Record<string, unknown> | null;
  configuration_default?: unknown;
  package_digest: string;
  source_kind: PluginSourceKind;
  source_ref: string;
  source_details: PluginSourceDetails;
  source_confirmation?: PluginSourceConfirmation;
  enabled: boolean;
  package_version: number;
  runtime_status: PluginRuntimeStatus;
  runtime_error_code: PluginRuntimeErrorCode | null;
  ui_surfaces: PluginSurface[];
  api_routes: string[];
  api_middleware: string[];
}

export type PluginSourceKind = "file" | "url" | "github_release" | "mounted";
export type PluginSourceConfirmation = "administrator_confirmed" | "gitops_mounted";
export type PluginSourceDetails =
  | { kind: "file"; filename: string }
  | { kind: "url"; url: string }
  | { kind: "github_release"; repository: string; tag: string; asset: string }
  | { kind: "mounted"; name: string };
export type PluginRuntimeStatus = "loaded" | "disabled" | "error";
export type PluginRuntimeErrorCode = "compile_failed" | "schema_invalid" | "interface_incompatible";
export type PluginContribution = "workspace_create_policy" | "configuration" | "ui_surfaces" | "api_routes" | "api_middleware";

export interface PluginSurface {
  id: string;
  title: string;
  placement: "admin_tab" | "workspace_detail";
  entrypoint: string;
  allowed_bridge_methods: string[];
}

export interface PluginInspectionManifest {
  id: string;
  name: string;
  version: string;
  description: string;
  wit_version: string;
}

export interface PluginInspection {
  inspection_id: string;
  expires_at: number;
  digest: string;
  size_bytes: number;
  source_kind: Exclude<PluginSourceKind, "mounted">;
  source_ref: string;
  source_confirmation?: PluginSourceConfirmation;
  manifest: PluginInspectionManifest;
  declared_contributions: PluginContribution[];
  current_package_version: number;
}

export interface PluginSurfaceSession {
  launch_url: string;
  expires_at: number;
  channel_nonce: string;
  allowed_bridge_methods: string[];
  bridge_url: string;
}

export interface PluginBridgeRequest {
  request_id: string;
  method: string;
  payload: unknown;
}

export type PluginApiRequestMethod = "GET" | "POST" | "PUT" | "PATCH" | "DELETE";

export interface PluginApiBridgeRequest {
  route_id: string;
  method: PluginApiRequestMethod;
  path: string;
  body?: unknown;
}

export interface PluginApiBridgeResponse {
  status: number;
  content_type: string;
  body: string;
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
