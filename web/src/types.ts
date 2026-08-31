export type Role = "system_admin" | "organization_admin" | "member";
export type WorkspaceState =
  | "provisioning"
  | "ready"
  | "stopping"
  | "stopped"
  | "starting"
  | "restarting"
  | "deleting"
  | "deleted"
  | "failed";
export type AccessMode = "internal" | "public";
export type InjectionScope = "organization" | "user" | "workspace";
export type InjectionKind =
  | "environment_variable"
  | "secret_file"
  | "config_file"
  | "ssh_public_key";

export interface Membership {
  organization_id: string;
  role: Role;
}

export interface Principal {
  user_id: string;
  display_name: string;
  avatar_url?: string | null;
  system_admin: boolean;
  memberships: Membership[];
}

export interface UserProfile {
  display_name: string;
  avatar_url: string | null;
}

export interface ApiKeySummary {
  id: string;
  name: string;
  prefix: string;
  last_used_at: number | null;
  created_at: number;
}

export type CreatedApiKey = ApiKeySummary & { token: string };

export interface Organization {
  id: string;
  name: string;
  created_at: number;
}

export interface UserSummary {
  id: string;
  display_name: string;
  system_admin: boolean;
  disabled: boolean;
  created_at: number;
}

export interface ImagePolicy {
  image: string;
  contract_version: number;
  enabled: boolean;
  created_at: number;
  updated_at: number;
}

export interface WorkspaceTemplate {
  id: string;
  organization_id: string | null;
  name: string;
  image: string;
  access_mode: AccessMode;
  resources: Resources;
  pod_requests: PodResourceRequest;
  ephemeral_storage_limit_mib: number | null;
  workspace_user: string;
  workspace_home: string;
  preserve_home_ownership: boolean;
  buildkit: boolean;
  storage_policy: WorkspaceStoragePolicy;
  cluster_access: boolean;
  required_node_names: string[];
  preferred_node_names: string[];
  node_selector: Record<string, string>;
  environment: Record<string, string>;
  yaml: string;
  enabled: boolean;
}

export interface WorkspaceStoragePolicy {
  runtime_tmp_memory_mib: number;
  build_scratch_gib: number;
  buildkit_cache_gib: number;
  codex_scratch_gib: number;
  home_reserve_mib: number | null;
}

export interface PodResourceRequest {
  cpu_millis: number;
  memory_mib: number;
  ephemeral_storage_mib: number | null;
}

export interface AuditRecord {
  id: string;
  actor_user_id: string | null;
  actor_display_name: string | null;
  organization_id: string | null;
  workspace_id: string | null;
  workspace_name: string | null;
  workspace_short_id: string | null;
  action: string;
  metadata: Record<string, unknown>;
  created_at: number;
}

export interface AuditPage {
  items: AuditRecord[];
  next_offset: number | null;
}

export interface ScalingStatus {
  database_mode: "sqlite" | "postgres";
  configured_replicas: number;
  schema_version: number;
  jobs: { pending: number; running: number; completed: number };
}

export interface WebhookSubscription {
  id: string;
  organization_id: string;
  url: string;
  event_prefix: string;
  enabled: boolean;
  created_at: number;
  updated_at: number;
}

export interface Resources {
  cpu_millis: number;
  memory_mib: number;
  gpu_count: number;
  disk_gib: number;
}

export interface Workspace {
  id: string;
  short_id: string;
  organization_id: string;
  owner_id: string;
  name: string;
  template_id: string | null;
  image: string;
  access_mode: AccessMode;
  pod_requests: PodResourceRequest;
  ephemeral_storage_limit_mib: number | null;
  workspace_user: string;
  workspace_home: string;
  preserve_home_ownership: boolean;
  buildkit: boolean;
  storage_policy: WorkspaceStoragePolicy;
  cluster_access: boolean;
  required_node_names: string[];
  preferred_node_names: string[];
  node_selector: Record<string, string>;
  environment: Record<string, string>;
  state: WorkspaceState;
  resources: Resources;
  generation: number;
  created_at: number;
  updated_at: number;
}

export interface WorkspaceResponse {
  workspace: Workspace;
  namespace: string;
  ssh_connection: WorkspaceSshConnection | null;
  ssh_host: string | null;
  ssh_port: number | null;
  ssh_command: string | null;
  ssh_config: string | null;
  web_shell_url: string | null;
  injection_sources: ResolvedInjection[];
  workspace_host_key: { algorithm: string; public_key: string; fingerprint: string } | null;
  jump_host_key: { algorithm: string; public_key: string; fingerprint: string } | null;
}

export interface WorkspaceSshConnection {
  display_name: string;
  alias: string;
  hostname: string;
  port: number;
  user: string;
  command: string;
  config: string;
  app: {
    display_name: string;
    hostname: string;
    ssh_port: number | null;
    port_strategy: "ssh_config";
  };
}

export interface WorkspaceRuntime {
  allocated: Resources;
  pvc_capacity: string | null;
  storage: {
    status: "available" | "stale" | "unavailable" | "disabled";
    used_bytes: number | null;
    capacity_bytes: number | null;
    available_bytes: number | null;
    observed_at: number | null;
    used_percent: number | null;
    pressure: "normal" | "warning" | "critical" | null;
  };
  metrics_available: boolean;
  pods: { name: string; phase: string | null; ready: boolean; restarts: number }[];
  metrics: { pod: string; container: string; cpu: string | null; memory: string | null }[];
  events: { reason: string | null; message: string | null; event_type: string | null; count: number | null; last_timestamp: string | null }[];
}

export interface WorkspaceRuntimeEntry {
  workspace_id: string;
  runtime: WorkspaceRuntime;
}

export interface CreateWorkspace {
  organization_id: string;
  owner_id: string;
  name: string;
  template_id: string;
  resources: Resources | null;
  organization_injection_refs: string[] | null;
  user_injection_refs: string[] | null;
}

export interface StoredInjection {
  key: string;
  kind: InjectionKind;
  target: string;
  scope: InjectionScope;
  scope_id: string;
  sensitive: boolean;
  locked: boolean;
  version: number;
  file_mode: number | null;
  owner: string | null;
  group: string | null;
  template_selector: string | null;
  labels: Record<string, string>;
  updated_at: number;
}

export interface InjectionDraft {
  key: string;
  kind: InjectionKind;
  target: string;
  value: { encoding: "utf8" | "base64"; value: string };
  sensitive: boolean;
  locked: boolean;
  version: number;
  file_mode: number | null;
  owner: string | null;
  group: string | null;
  template_selector: string | null;
  labels: Record<string, string>;
}

export interface ResolvedInjection {
  key: string;
  kind: InjectionKind;
  target: string;
  source: InjectionScope;
  sensitive: boolean;
  locked: boolean;
  version: number;
}

export interface WebShellTicket {
  ticket: string;
  workspace_id: string;
  expires_at: number;
  web_shell_url: string;
}

export interface ApiFailure {
  error?: { code?: string; message?: string };
}
