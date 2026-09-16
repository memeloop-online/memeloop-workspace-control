import { parse, stringify } from "yaml";

import type { AccessMode, EgressPolicy, WorkspacePlacement, WorkspaceStoragePolicy, WorkspaceTemplate } from "../types";

export const DEFAULT_STORAGE_POLICY: WorkspaceStoragePolicy = {
  temporary_storage_gib: 10,
};

export const DEFAULT_PLACEMENT: WorkspacePlacement = {
  allowed_node_pools: ["default"],
  default_node_pool: "default",
};

export interface NumericPolicy {
  min: number;
  step: number;
  max: number;
}

export const TEMPLATE_NUMBER_POLICIES = {
  cpu: { min: 100, step: 100, max: 256_000 },
  memory: { min: 128, step: 128, max: 1_048_576 },
  requestCpu: { min: 100, step: 100, max: 256_000 },
  requestMemory: { min: 128, step: 128, max: 1_048_576 },
  gpu: { min: 0, step: 1, max: 64 },
  disk: { min: 1, step: 1, max: 16_384 },
  temporaryStorage: { min: 1, step: 1, max: 2_048 },
  desktopPort: { min: 1_024, step: 1, max: 65_535 },
} as const satisfies Record<string, NumericPolicy>;

export interface TemplateStoragePolicyDraft {
  temporary_storage_gib: string;
}

export interface TemplateDraft {
  name: string;
  image: string;
  accessMode: AccessMode;
  cpu: string;
  memory: string;
  gpu: string;
  disk: string;
  requestCpu: string;
  requestMemory: string;
  user: string;
  home: string;
  buildkit: boolean;
  storagePolicy: TemplateStoragePolicyDraft;
  clusterAccess: boolean;
  egressPolicy: EgressPolicy;
  runtimeClassName: string;
  allowedNodePools: string[];
  defaultNodePool: string;
  desktopEnabled: boolean;
  desktopPort: string;
  desktopDisplayName: string;
}

type TemplateFieldSchema = { readonly [field: string]: TemplateFieldSchema | null };

const TEMPLATE_FIELD_SCHEMA: TemplateFieldSchema = {
  apiVersion: null,
  kind: null,
  metadata: { name: null },
  spec: {
    image: null,
    access_mode: null,
    resources: {
      cpu_millis: null,
      memory_mib: null,
      gpu_count: null,
      disk_gib: null,
    },
    pod_requests: {
      cpu_millis: null,
      memory_mib: null,
    },
    workspace_user: null,
    workspace_home: null,
    buildkit: null,
    storage_policy: {
      temporary_storage_gib: null,
    },
    cluster_access: null,
    egress_policy: null,
    runtime_class_name: null,
    placement: {
      allowed_node_pools: null,
      default_node_pool: null,
    },
    desktop: {
      internal_port: null,
      display_name: null,
    },
  },
};

export type TemplateDraftErrorCode =
  | "invalid_template_number"
  | "resource_request_exceeds_limit"
  | "invalid_node_pool_placement";

export class TemplateDraftError extends Error {
  readonly code: TemplateDraftErrorCode;

  constructor(code: TemplateDraftErrorCode) {
    super(code);
    this.code = code;
  }
}

export function emptyTemplateDraft(): TemplateDraft {
  return {
    name: "",
    image: "",
    accessMode: "internal",
    cpu: "2000",
    memory: "4096",
    gpu: "0",
    disk: "50",
    requestCpu: "500",
    requestMemory: "1024",
    user: "workspace",
    home: "/workspace",
    buildkit: false,
    storagePolicy: storagePolicyDraft(DEFAULT_STORAGE_POLICY),
    clusterAccess: false,
    egressPolicy: "unrestricted",
    runtimeClassName: "",
    allowedNodePools: [...DEFAULT_PLACEMENT.allowed_node_pools],
    defaultNodePool: DEFAULT_PLACEMENT.default_node_pool,
    desktopEnabled: false,
    desktopPort: "6080",
    desktopDisplayName: "",
  };
}

export function templateDraftToYaml(draft: TemplateDraft): string {
  const cpu = parseRequiredNumber(draft.cpu, TEMPLATE_NUMBER_POLICIES.cpu);
  const memory = parseRequiredNumber(draft.memory, TEMPLATE_NUMBER_POLICIES.memory);
  const gpu = parseRequiredNumber(draft.gpu, TEMPLATE_NUMBER_POLICIES.gpu);
  const disk = parseRequiredNumber(draft.disk, TEMPLATE_NUMBER_POLICIES.disk);
  const requestCpu = parseRequiredNumber(draft.requestCpu, TEMPLATE_NUMBER_POLICIES.requestCpu);
  const requestMemory = parseRequiredNumber(draft.requestMemory, TEMPLATE_NUMBER_POLICIES.requestMemory);
  const desktopPort = draft.desktopEnabled ? parseDesktopPort(draft.desktopPort) : null;
  const storagePolicy: WorkspaceStoragePolicy = {
    temporary_storage_gib: parseRequiredNumber(draft.storagePolicy.temporary_storage_gib, TEMPLATE_NUMBER_POLICIES.temporaryStorage),
  };
  const placement = placementFromDraft(draft);

  if (requestCpu > cpu || requestMemory > memory) {
    throw new TemplateDraftError("resource_request_exceeds_limit");
  }

  const spec: Record<string, unknown> = {
    image: draft.image,
    access_mode: draft.accessMode,
    resources: { cpu_millis: cpu, memory_mib: memory, gpu_count: gpu, disk_gib: disk },
    pod_requests: { cpu_millis: requestCpu, memory_mib: requestMemory },
    workspace_user: draft.user,
    workspace_home: draft.home,
    buildkit: draft.buildkit,
    storage_policy: storagePolicy,
    cluster_access: draft.clusterAccess,
    egress_policy: draft.egressPolicy,
    placement: {
      allowed_node_pools: placement.allowed_node_pools,
      default_node_pool: placement.default_node_pool,
    },
  };
  if (draft.runtimeClassName.trim()) spec.runtime_class_name = draft.runtimeClassName.trim();
  if (desktopPort !== null) {
    spec.desktop = {
      internal_port: desktopPort,
      ...(draft.desktopDisplayName.trim() ? { display_name: draft.desktopDisplayName.trim() } : {}),
    };
  }
  return stringify({ apiVersion: "workspace.memeloop.dev/v1", kind: "WorkspaceTemplate", metadata: { name: draft.name }, spec }, { lineWidth: 0 });
}

function placementFromDraft(draft: TemplateDraft): WorkspacePlacement {
  const allowed = [...new Set(draft.allowedNodePools.map((pool) => pool.trim()).filter(Boolean))];
  const defaultPool = draft.defaultNodePool.trim();
  if (
    allowed.length === 0
    || allowed.length > 32
    || allowed.some((pool) => !isValidNodePoolName(pool))
    || !isValidNodePoolName(defaultPool)
    || !allowed.includes(defaultPool)
  ) {
    throw new TemplateDraftError("invalid_node_pool_placement");
  }
  return { allowed_node_pools: allowed, default_node_pool: defaultPool };
}

function isValidNodePoolName(value: string): boolean {
  return value.length <= 63
    && (/^[a-z]$/u.test(value) || /^[a-z][a-z0-9-]*[a-z0-9]$/u.test(value));
}

export function templateDraftFromYaml(yaml: string): TemplateDraft {
  const value = knownFields(parse(yaml), TEMPLATE_FIELD_SCHEMA, "document");
  const metadata = requiredRecord(value.metadata);
  const spec = requiredRecord(value.spec);
  const resources = requiredRecord(spec.resources);
  const podRequests = requiredRecord(spec.pod_requests);
  const storagePolicy = spec.storage_policy === undefined
    ? DEFAULT_STORAGE_POLICY
    : parseStoragePolicy(spec.storage_policy);
  const placement = spec.placement === undefined
    ? DEFAULT_PLACEMENT
    : parsePlacement(spec.placement);
  const desktop = spec.desktop === undefined ? null : parseDesktop(spec.desktop);
  if (!metadata.name) throw new Error("Invalid WorkspaceTemplate YAML");
  return {
    name: String(metadata.name),
    image: String(spec.image ?? ""),
    accessMode: spec.access_mode === "public" ? "public" : "internal",
    cpu: numberText(resources.cpu_millis),
    memory: numberText(resources.memory_mib),
    gpu: numberText(resources.gpu_count),
    disk: numberText(resources.disk_gib),
    requestCpu: numberText(podRequests.cpu_millis),
    requestMemory: numberText(podRequests.memory_mib),
    user: String(spec.workspace_user ?? ""),
    home: String(spec.workspace_home ?? ""),
    buildkit: Boolean(spec.buildkit),
    storagePolicy: storagePolicyDraft(storagePolicy),
    clusterAccess: Boolean(spec.cluster_access),
    egressPolicy: spec.egress_policy === "internet_only" ? "internet_only" : "unrestricted",
    runtimeClassName: String(spec.runtime_class_name ?? ""),
    allowedNodePools: placement.allowed_node_pools,
    defaultNodePool: placement.default_node_pool,
    desktopEnabled: desktop !== null,
    desktopPort: desktop === null ? "6080" : String(desktop.internal_port),
    desktopDisplayName: desktop?.display_name ?? "",
  };
}

function parsePlacement(value: unknown): WorkspacePlacement {
  const record = requiredRecord(value);
  const allowed = record.allowed_node_pools;
  const defaultPool = record.default_node_pool;
  if (
    !Array.isArray(allowed)
    || allowed.some((pool) => typeof pool !== "string")
    || typeof defaultPool !== "string"
  ) {
    throw new Error("Invalid WorkspaceTemplate YAML: placement requires string node pool names");
  }
  return { allowed_node_pools: allowed as string[], default_node_pool: defaultPool };
}

function parseDesktop(value: unknown): { internal_port: number; display_name?: string } {
  const desktop = requiredRecord(value);
  const port = desktop.internal_port;
  if (typeof port !== "number" || !Number.isSafeInteger(port) || !isDesktopPort(port)) {
    throw new Error("Invalid WorkspaceTemplate YAML: desktop.internal_port is unavailable for browser desktop access");
  }
  if (desktop.display_name !== undefined && typeof desktop.display_name !== "string") {
    throw new Error("Invalid WorkspaceTemplate YAML: desktop.display_name must be a string");
  }
  return {
    internal_port: port,
    ...(typeof desktop.display_name === "string" && desktop.display_name ? { display_name: desktop.display_name } : {}),
  };
}

function parseDesktopPort(value: string): number {
  const port = parseRequiredNumber(value, TEMPLATE_NUMBER_POLICIES.desktopPort);
  if (!isDesktopPort(port)) throw new TemplateDraftError("invalid_template_number");
  return port;
}

function isDesktopPort(port: number): boolean {
  return port >= TEMPLATE_NUMBER_POLICIES.desktopPort.min
    && port <= TEMPLATE_NUMBER_POLICIES.desktopPort.max
    && ![22, 2222, 7681, 8080, 8081, 8443, 3389].includes(port);
}

function parseStoragePolicy(value: unknown): WorkspaceStoragePolicy {
  const policy = requiredRecord(value);
  return {
    temporary_storage_gib: Number(policy.temporary_storage_gib ?? DEFAULT_STORAGE_POLICY.temporary_storage_gib),
  };
}

function requiredRecord(value: unknown): Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("Invalid WorkspaceTemplate YAML");
  }
  return value as Record<string, unknown>;
}

function knownFields(value: unknown, schema: TemplateFieldSchema, path: string): Record<string, unknown> {
  const record = requiredRecord(value);
  for (const key of Object.keys(record)) {
    if (!Object.prototype.hasOwnProperty.call(schema, key)) {
      throw new Error(`Invalid WorkspaceTemplate YAML: unknown field ${path}.${key}`);
    }
    const nested = schema[key];
    if (nested !== null) knownFields(record[key], nested, `${path}.${key}`);
  }
  return record;
}

function storagePolicyDraft(policy: WorkspaceStoragePolicy): TemplateStoragePolicyDraft {
  return {
    temporary_storage_gib: String(policy.temporary_storage_gib),
  };
}

export function templateDraftFromTemplate(template: WorkspaceTemplate): TemplateDraft {
  return templateDraftFromYaml(template.yaml);
}

function parseRequiredNumber(value: string, policy: NumericPolicy): number {
  const parsed = Number(value);
  if (!value || !Number.isSafeInteger(parsed) || parsed < policy.min || parsed > policy.max || (parsed - policy.min) % policy.step !== 0) {
    throw new TemplateDraftError("invalid_template_number");
  }
  return parsed;
}

function numberText(value: unknown) {
  return value === undefined || value === null ? "0" : String(value);
}
