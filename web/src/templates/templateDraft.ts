import { parse, stringify } from "yaml";

import type { AccessMode, WorkspaceStoragePolicy, WorkspaceTemplate } from "../types";

export const DEFAULT_STORAGE_POLICY: WorkspaceStoragePolicy = {
  runtime_tmp_memory_mib: 512,
  build_scratch_gib: 12,
  buildkit_cache_gib: 8,
  codex_scratch_gib: 2,
  home_reserve_mib: 1024,
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
  ephemeral: { min: 128, step: 128, max: 1_048_576 },
  gpu: { min: 0, step: 1, max: 64 },
  disk: { min: 1, step: 1, max: 16_384 },
  runtimeTmpMemory: { min: 64, step: 64, max: 4_096 },
  buildScratch: { min: 1, step: 1, max: 256 },
  buildkitCache: { min: 1, step: 1, max: 256 },
  codexScratch: { min: 1, step: 1, max: 32 },
  homeReserve: { min: 64, step: 64, max: 4_096 },
} as const satisfies Record<string, NumericPolicy>;

export interface TemplateStoragePolicyDraft {
  runtime_tmp_memory_mib: string;
  build_scratch_gib: string;
  buildkit_cache_gib: string;
  codex_scratch_gib: string;
  home_reserve_mib: string;
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
  requestEphemeral: string;
  limitEphemeral: string;
  user: string;
  home: string;
  preserveHome: boolean;
  buildkit: boolean;
  storagePolicy: TemplateStoragePolicyDraft;
  clusterAccess: boolean;
  requiredNodes: string;
  preferredNodes: string;
  nodeSelector: string;
}

export type TemplateDraftErrorCode =
  | "invalid_template_number"
  | "resource_request_exceeds_limit"
  | "ephemeral_request_exceeds_limit";

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
    requestEphemeral: "2048",
    limitEphemeral: "14592",
    user: "workspace",
    home: "/workspace",
    preserveHome: false,
    buildkit: false,
    storagePolicy: storagePolicyDraft(DEFAULT_STORAGE_POLICY),
    clusterAccess: false,
    requiredNodes: "",
    preferredNodes: "",
    nodeSelector: "",
  };
}

export function templateDraftToYaml(draft: TemplateDraft): string {
  const cpu = parseRequiredNumber(draft.cpu, TEMPLATE_NUMBER_POLICIES.cpu);
  const memory = parseRequiredNumber(draft.memory, TEMPLATE_NUMBER_POLICIES.memory);
  const gpu = parseRequiredNumber(draft.gpu, TEMPLATE_NUMBER_POLICIES.gpu);
  const disk = parseRequiredNumber(draft.disk, TEMPLATE_NUMBER_POLICIES.disk);
  const requestCpu = parseRequiredNumber(draft.requestCpu, TEMPLATE_NUMBER_POLICIES.requestCpu);
  const requestMemory = parseRequiredNumber(draft.requestMemory, TEMPLATE_NUMBER_POLICIES.requestMemory);
  const requestEphemeral = parseOptionalNumber(draft.requestEphemeral, TEMPLATE_NUMBER_POLICIES.ephemeral);
  const limitEphemeral = parseOptionalNumber(draft.limitEphemeral, TEMPLATE_NUMBER_POLICIES.ephemeral);
  const storagePolicy: WorkspaceStoragePolicy = {
    runtime_tmp_memory_mib: parseRequiredNumber(draft.storagePolicy.runtime_tmp_memory_mib, TEMPLATE_NUMBER_POLICIES.runtimeTmpMemory),
    build_scratch_gib: parseRequiredNumber(draft.storagePolicy.build_scratch_gib, TEMPLATE_NUMBER_POLICIES.buildScratch),
    buildkit_cache_gib: parseRequiredNumber(draft.storagePolicy.buildkit_cache_gib, TEMPLATE_NUMBER_POLICIES.buildkitCache),
    codex_scratch_gib: parseRequiredNumber(draft.storagePolicy.codex_scratch_gib, TEMPLATE_NUMBER_POLICIES.codexScratch),
    home_reserve_mib: parseOptionalNumber(draft.storagePolicy.home_reserve_mib, TEMPLATE_NUMBER_POLICIES.homeReserve),
  };

  if (requestCpu > cpu || requestMemory > memory) {
    throw new TemplateDraftError("resource_request_exceeds_limit");
  }
  if (requestEphemeral !== null && limitEphemeral !== null && requestEphemeral > limitEphemeral) {
    throw new TemplateDraftError("ephemeral_request_exceeds_limit");
  }
  if (storagePolicy.home_reserve_mib !== null && (storagePolicy.home_reserve_mib >= disk * 1_024 || storagePolicy.home_reserve_mib * 10 > disk * 1_024)) {
    throw new TemplateDraftError("invalid_template_number");
  }

  const spec: Record<string, unknown> = {
    image: draft.image,
    access_mode: draft.accessMode,
    resources: { cpu_millis: cpu, memory_mib: memory, gpu_count: gpu, disk_gib: disk },
    pod_requests: { cpu_millis: requestCpu, memory_mib: requestMemory },
    workspace_user: draft.user,
    workspace_home: draft.home,
    preserve_home_ownership: draft.preserveHome,
    buildkit: draft.buildkit,
    storage_policy: storagePolicy,
    cluster_access: draft.clusterAccess,
  };
  if (requestEphemeral !== null) (spec.pod_requests as Record<string, unknown>).ephemeral_storage_mib = requestEphemeral;
  if (limitEphemeral !== null) spec.ephemeral_storage_limit_mib = limitEphemeral;
  const required = csv(draft.requiredNodes); if (required.length) spec.required_node_names = required;
  const preferred = csv(draft.preferredNodes); if (preferred.length) spec.preferred_node_names = preferred;
  const selector = pairs(draft.nodeSelector); if (Object.keys(selector).length) spec.node_selector = selector;
  return stringify({ apiVersion: "workspace.memeloop.dev/v1", kind: "WorkspaceTemplate", metadata: { name: draft.name }, spec }, { lineWidth: 0 });
}

export function templateDraftFromYaml(yaml: string): TemplateDraft {
  const value = parse(yaml) as Record<string, any>;
  if (!value?.metadata?.name || !value?.spec) throw new Error("Invalid WorkspaceTemplate YAML");
  const spec = value.spec;
  return {
    name: String(value.metadata.name),
    image: String(spec.image ?? ""),
    accessMode: spec.access_mode === "public" ? "public" : "internal",
    cpu: numberText(spec.resources?.cpu_millis),
    memory: numberText(spec.resources?.memory_mib),
    gpu: numberText(spec.resources?.gpu_count),
    disk: numberText(spec.resources?.disk_gib),
    requestCpu: numberText(spec.pod_requests?.cpu_millis),
    requestMemory: numberText(spec.pod_requests?.memory_mib),
    requestEphemeral: optionalNumberText(spec.pod_requests?.ephemeral_storage_mib),
    limitEphemeral: optionalNumberText(spec.ephemeral_storage_limit_mib),
    user: String(spec.workspace_user ?? ""),
    home: String(spec.workspace_home ?? ""),
    preserveHome: Boolean(spec.preserve_home_ownership ?? spec.preserve_home_root),
    buildkit: Boolean(spec.buildkit),
    storagePolicy: storagePolicyDraft(parseStoragePolicy(spec.storage_policy)),
    clusterAccess: Boolean(spec.cluster_access),
    requiredNodes: (spec.required_node_names ?? []).join(", "),
    preferredNodes: (spec.preferred_node_names ?? []).join(", "),
    nodeSelector: formatPairs(spec.node_selector),
  };
}

function parseStoragePolicy(value: unknown): WorkspaceStoragePolicy {
  const policy = value && typeof value === "object" ? value as Partial<WorkspaceStoragePolicy> : {};
  return {
    runtime_tmp_memory_mib: Number(policy.runtime_tmp_memory_mib ?? DEFAULT_STORAGE_POLICY.runtime_tmp_memory_mib),
    build_scratch_gib: Number(policy.build_scratch_gib ?? DEFAULT_STORAGE_POLICY.build_scratch_gib),
    buildkit_cache_gib: Number(policy.buildkit_cache_gib ?? DEFAULT_STORAGE_POLICY.buildkit_cache_gib),
    codex_scratch_gib: Number(policy.codex_scratch_gib ?? DEFAULT_STORAGE_POLICY.codex_scratch_gib),
    home_reserve_mib: policy.home_reserve_mib == null ? null : Number(policy.home_reserve_mib),
  };
}

function storagePolicyDraft(policy: WorkspaceStoragePolicy): TemplateStoragePolicyDraft {
  return {
    runtime_tmp_memory_mib: String(policy.runtime_tmp_memory_mib),
    build_scratch_gib: String(policy.build_scratch_gib),
    buildkit_cache_gib: String(policy.buildkit_cache_gib),
    codex_scratch_gib: String(policy.codex_scratch_gib),
    home_reserve_mib: optionalNumberText(policy.home_reserve_mib),
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

function parseOptionalNumber(value: string, policy: NumericPolicy): number | null {
  return value === "" ? null : parseRequiredNumber(value, policy);
}

function numberText(value: unknown) {
  return value === undefined || value === null ? "0" : String(value);
}

function optionalNumberText(value: unknown) {
  return value === undefined || value === null ? "" : String(value);
}

function csv(value: string) {
  return value.split(",").map((item) => item.trim()).filter(Boolean);
}

function pairs(value: string) {
  return Object.fromEntries(value.split("\n").map((line) => line.trim()).filter(Boolean).map((line) => {
    const index = line.indexOf("=");
    if (index < 1) throw new Error(`Expected KEY=value: ${line}`);
    return [line.slice(0, index).trim(), line.slice(index + 1)];
  }));
}

function formatPairs(value: unknown) {
  return value && typeof value === "object"
    ? Object.entries(value as Record<string, unknown>).map(([key, item]) => `${key}=${String(item)}`).join("\n")
    : "";
}
