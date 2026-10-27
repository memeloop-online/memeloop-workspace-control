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
export type EgressPolicy = "unrestricted" | "internet_only";
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
  api_key_scopes: ApiKeyScope[];
  api_key_expires_at: number | null;
  allowed_template_ids: string[] | null;
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
  scopes: ApiKeyScope[];
  expires_at: number | null;
  allowed_template_ids: string[] | null;
  revoked_at: number | null;
}

export interface ApiKeyPage {
  items: AdminApiKey[];
  next_cursor: string | null;
}

/** Plaintext is deliberately returned only by the administrator key endpoint. */
export type AdminApiKey = ApiKeySummary & { token: string | null };

export type ApiKeyScope =
  | "manage_api_keys"
  | "manage_system"
  | "manage_organization"
  | "manage_members"
  | "manage_locked_injections"
  | "create_workspace"
  | "read_workspace"
  | "connect_workspace"
  | "change_workspace_state"
  | "delete_workspace";

export type CreatedApiKey = ApiKeySummary & { token: string };

export interface Organization {
  id: string;
  name: string;
  created_at: number;
}

export interface OrganizationPage {
  items: Organization[];
  next_cursor: string | null;
}

export interface UserSummary {
  id: string;
  display_name: string;
  system_admin: boolean;
  disabled: boolean;
  created_at: number;
  /** Returned when the admin user page is scoped to an organization. */
  membership_role?: Role | null;
}

export interface CreateUserInput {
  display_name: string;
  token: string;
  system_admin: boolean;
  scopes: ApiKeyScope[];
  expires_at: number;
  organization_id?: string;
  organization_role?: Exclude<Role, "system_admin">;
}

export interface UserPage {
  items: UserSummary[];
  next_cursor: string | null;
}

export interface MembershipSummary {
  user: UserSummary;
  role: Role;
}

export interface MembershipPage {
  items: MembershipSummary[];
  next_cursor: string | null;
}

export interface WorkspacePage {
  items: WorkspaceResponse[];
  next_cursor: string | null;
  summary: WorkspaceSummary;
}

/** Creation-time ordering for the workspace list; the backend defaults to "newest". */
export type WorkspaceSort = "newest" | "oldest";

export interface WorkspaceSummary {
  total_count: number;
  requested: QuotaResources;
  temporary_requested_gib: number;
  state_counts: Partial<Record<WorkspaceState, number>>;
}

export interface QuotaResources {
  cpu_millis: number;
  memory_mib: number;
  gpu_count: number;
  disk_gib: number;
}

export interface AvailableNodePool {
  name: string;
  display_name: string;
}

/** Private node selection rules stored by a system-managed run location. */
export interface NodePoolPlacement {
  selector: Record<string, string>;
  required_hosts: string[];
  preferred_hosts: string[];
}

/** Administrator view of a run location; the public list stays AvailableNodePool. */
export interface NodePool {
  name: string;
  display_name: string;
  placement: NodePoolPlacement;
  enabled: boolean;
  created_at: number;
  updated_at: number;
}

export interface PutNodePoolInput {
  display_name: string;
  placement: NodePoolPlacement;
  enabled: boolean;
}

export interface WorkspacePlacement {
  allowed_node_pools: string[];
  default_node_pool: string;
}

export type UsageAvailability = "available" | "unavailable" | "unknown";

/** Organization-wide runtime observation, independent of workspace paging. */
export interface OrganizationUsageSummary {
  total_count: number;
  state_counts: Partial<Record<WorkspaceState, number>>;
  requested: QuotaResources;
  temporary_requested_gib: number;
  actual: { cpu_millis: number | null; memory_mib: number | null; disk_bytes: number | null; temporary_bytes: number | null };
  observed_at: number | null;
  availability: { cpu: UsageAvailability; memory: UsageAvailability; disk: UsageAvailability; temporary: UsageAvailability };
  coverage: { total_workspaces: number; eligible_workspaces: number; template_label_coverage: "complete" | "incomplete" };
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
  /** Missing template values inherit at creation; missing workspace snapshots retain ratio 1. */
  cpu_overcommit_ratio?: number | null;
  workspace_user: string;
  workspace_home: string;
  buildkit: boolean;
  storage_policy: WorkspaceStoragePolicy;
  cluster_access: boolean;
  egress_policy: EgressPolicy;
  runtime_class_name: string | null;
  placement: WorkspacePlacement;
  desktop?: WorkspaceDesktopTemplate | null;
  yaml: string;
  enabled: boolean;
}

/** Optional browser desktop endpoint provisioned with a workspace template. */
export interface WorkspaceDesktopTemplate {
  internal_port: number;
  display_name?: string;
}

export interface WorkspaceStoragePolicy {
  temporary_storage_gib: number;
}

export interface PodResourceRequest {
  cpu_millis: number;
  memory_mib: number;
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

export interface ScalingSettingsPatch {
  job_worker_concurrency?: number;
  cpu_overcommit_ratio?: number;
}

export interface ScalingStatus {
  database_mode: "sqlite" | "postgres";
  configured_replicas: number;
  schema_version: number;
  job_worker_concurrency: number;
  cpu_overcommit_ratio?: number;
  jobs: { pending: number; running: number; completed: number; failed: number };
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
  node_pool: string;
  image: string;
  access_mode: AccessMode;
  pod_requests: PodResourceRequest;
  /** Missing template values inherit at creation; missing workspace snapshots retain ratio 1. */
  cpu_overcommit_ratio?: number | null;
  workspace_user: string;
  workspace_home: string;
  buildkit: boolean;
  storage_policy: WorkspaceStoragePolicy;
  cluster_access: boolean;
  egress_policy: EgressPolicy;
  runtime_class_name: string | null;
  placement: WorkspacePlacement;
  state: WorkspaceState;
  resources: Resources;
  home_volume_binding?: WorkspaceHomeVolumeBinding | null;
  scratch_volume_binding?: WorkspaceHomeVolumeBinding | null;
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
  desktop?: WorkspaceDesktopConnection | null;
}

/** A browser desktop endpoint returned with a workspace response. */
export interface WorkspaceDesktopConnection {
  mapping_id: string;
  display_name: string;
  status: "provisioning" | "ready" | "failed" | "deleting" | string;
  https_url: string | null;
}

export interface WorkspaceSshConnection {
  display_name: string;
  alias: string;
  hostname: string;
  port: number;
  user: string;
  command: string;
  config: string;
  /** Human-readable host key alias; matches `alias` and the config HostKeyAlias line. */
  host_key_alias?: string;
  /** Ready-to-append known_hosts line keyed by the host key alias. */
  known_hosts_entry?: string | null;
  app: {
    display_name: string;
    hostname: string;
    ssh_port: number | null;
    port_strategy: "ssh_config";
  };
}

export interface WorkspaceRuntime {
  allocated: Resources;
  persistent_storage: WorkspaceStorageTelemetry;
  temporary_storage: WorkspaceStorageTelemetry;
  metrics_available: boolean;
  /** The Kubernetes node hosting the active workspace Pod, when scheduled. */
  node_name: string | null;
  pods: WorkspaceRuntimePod[];
  metrics: { pod: string; container: string; cpu: string | null; memory: string | null }[];
  events: WorkspaceRuntimeEvent[];
}

export interface WorkspaceRuntimePod {
  name: string;
  phase: string | null;
  ready: boolean;
  restarts: number;
  /** Startup milestones and stage timings, when observed. */
  startup?: WorkspacePodStartup | null;
  /** Per-container states, when observed. */
  containers?: WorkspaceContainerState[];
}

export interface WorkspacePodStartup {
  created_at: string | null;
  scheduled_at: string | null;
  containers_started_at: string | null;
  initialized_at: string | null;
  ready_at: string | null;
  total_seconds: number | null;
  stages: WorkspacePodStartupStage[];
}

export interface WorkspacePodStartupStage {
  name: string;
  started_at: string | null;
  finished_at: string | null;
  duration_seconds: number | null;
}

export interface WorkspaceContainerState {
  name: string;
  ready: boolean;
  restarts: number;
  state: string;
  reason: string | null;
  exit_code: number | null;
}

export interface WorkspaceRuntimeEvent {
  id: string;
  category: "disk_pressure" | "evicted" | "temporary_storage_provisioning" | "temporary_storage_attachment" | "volume_unavailable" | "ephemeral_storage" | "memory_pressure" | "pid_pressure" | "container_backoff" | "image_pull" | "scheduling" | "health_check" | "other";
  count: number | null;
  observed_at: string | null;
  last_observed_at: string | null;
  /** Typed, bounded facts parsed from the source event; raw messages are never exposed. Historical rows carry an object with null scalar fields, empty blockers, and no diagnostic. */
  details: RuntimeEventDetails;
}

export interface RuntimeEventDetails {
  event_uid: string | null;
  pod_uid: string | null;
  /** Short machine reason such as BackOff or FailedScheduling; never a raw event message. */
  reason: string | null;
  container: RuntimeContainer | null;
  /** Safe known scheduler causes; raw scheduler messages are never exposed. Only meaningful for scheduling events. */
  scheduling_blockers: SchedulingBlocker[];
  diagnostic: RuntimeDiagnostic | null;
}

export type RuntimeContainer =
  | "workspace"
  | "ttyd"
  | "buildkitd"
  | "workspace-scratch-init"
  | "workspace-bootstrap"
  | "buildkit-bootstrap";

export type RuntimeDiagnostic =
  | { kind: "container_local_ephemeral_limit"; container: RuntimeContainer; limit_bytes: number }
  | { kind: "pod_local_ephemeral_limit"; limit_bytes: number }
  | { kind: "node_resource_pressure"; resource: "ephemeral_storage" | "memory" | "pids" };

export type SchedulingBlocker =
  | "cpu_capacity"
  | "memory_capacity"
  | "storage_capacity"
  | "node_affinity"
  | "volume_affinity"
  | "taint"
  | "node_unavailable";

export interface WorkspaceStorageTelemetry {
  configured_bytes: number;
  used_bytes: number | null;
  capacity_bytes: number | null;
  available_bytes: number | null;
  observed_at: number | null;
  used_percent: number | null;
  pressure: "normal" | "warning" | "critical" | null;
  coverage: "exact" | "stale" | "unavailable" | "disabled";
  backing: "persistent_volume" | "ephemeral_volume" | "unknown";
}

export interface StorageShrinkRequest {
  home_gib?: number | null;
  temporary_storage_gib?: number | null;
  expected_generation: number;
  reclaim_source?: boolean;
}

export type StorageShrinkPhase =
  | "queued"
  | "stopping"
  | "measuring"
  | "copying"
  | "verifying"
  | "switching"
  | "resuming"
  | "rolling_back"
  | "completed"
  | "failed"
  | "reclaiming"
  | "reclaimed";

export interface WorkspaceHomeVolumeBinding {
  namespace: string;
  claim_name: string;
  claim_uid: string;
  capacity_gib: number;
}

export interface StorageVolumeIdentity {
  pv_name: string;
  pv_uid: string;
  csi_driver: string;
  csi_volume_handle: string;
  storage_class_name: string;
  longhorn_namespace: string | null;
  longhorn_volume_uid: string | null;
}

export interface StorageShrinkVolume {
  target_gib: number | null;
  source: WorkspaceHomeVolumeBinding | null;
  destination: WorkspaceHomeVolumeBinding | null;
  used_bytes: number | null;
  used_inodes: number | null;
  verified: boolean;
  source_volume: StorageVolumeIdentity | null;
  destination_volume: StorageVolumeIdentity | null;
}

export interface StorageShrinkOperation {
  id: string;
  workspace_id: string;
  phase: StorageShrinkPhase;
  error: string | null;
  home: StorageShrinkVolume;
  temporary: StorageShrinkVolume;
  resume: boolean;
  created_at: number;
  updated_at: number;
  actor_user_id: string;
  request: StorageShrinkRequest;
  original_home_gib: number;
  original_temporary_storage_gib: number;
  switched: boolean;
  sources_retained: boolean;
  reclaimable: boolean;
  helper_pod_uids: Record<string, string>;
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
  node_pool?: string | null;
  resources: Resources | null;
  temporary_storage_gib?: number | null;
  organization_injection_refs: string[] | null;
  user_injection_refs: string[] | null;
  inline_workspace_injections?: InjectionDraft[];
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
  /** Present only for non-sensitive items; sensitive items return null or omit it. */
  value?: { encoding: "utf8" | "base64"; value: string } | null;
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
