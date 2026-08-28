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
export type RuntimeProfile =
  | "standard"
  | "rust_dev"
  | "node_dev"
  | "maintainance";
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
  system_admin: boolean;
  memberships: Membership[];
}

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
  runtime_profile: RuntimeProfile;
  access_mode: AccessMode;
  resources: Resources;
  enabled: boolean;
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
  runtime_profile: RuntimeProfile;
  access_mode: AccessMode;
  state: WorkspaceState;
  resources: Resources;
  generation: number;
  created_at: number;
  updated_at: number;
}

export interface WorkspaceResponse {
  workspace: Workspace;
  namespace: string;
  ssh_host: string | null;
  ssh_port: number | null;
  ssh_command: string | null;
  ssh_config: string | null;
  web_shell_url: string | null;
  injection_sources: ResolvedInjection[];
  workspace_host_key: { algorithm: string; public_key: string; fingerprint: string } | null;
  jump_host_key: { algorithm: string; public_key: string; fingerprint: string } | null;
}

export interface WorkspaceRuntime {
  allocated: Resources;
  pvc_capacity: string | null;
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
  template_id: string | null;
  organization_injection_refs: string[] | null;
  user_injection_refs: string[] | null;
  image: string;
  runtime_profile: RuntimeProfile;
  access_mode: AccessMode;
  resources: Resources;
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
