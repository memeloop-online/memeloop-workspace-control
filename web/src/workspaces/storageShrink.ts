import type { MessageKey } from "../i18n";
import type {
  StorageShrinkOperation,
  StorageShrinkPhase,
  StorageShrinkRequest,
  StorageShrinkVolume,
  StorageVolumeIdentity,
  WorkspaceHomeVolumeBinding,
  WorkspaceState,
} from "../types";

type OperationStatus = Pick<StorageShrinkOperation, "phase" | "reclaimable" | "sources_retained">;

const GIB_BYTES = 1024 ** 3;

export const TEMPORARY_STORAGE_MAX_GIB = 2_048;

export type ShrinkFieldError = "invalid" | "notSmaller" | "aboveMax" | "belowUsage";

export interface ShrinkVolumeContext {
  currentGiB: number;
  usedBytes: number | null;
}

export interface ShrinkValidation {
  homeError?: ShrinkFieldError;
  temporaryError?: ShrinkFieldError;
  empty: boolean;
  targets: { home_gib?: number; temporary_storage_gib?: number } | null;
}

export function parseGiBTarget(raw: string): number | null | undefined {
  const trimmed = raw.trim();
  if (trimmed === "") return undefined;
  if (!/^\d+$/.test(trimmed)) return null;
  const value = Number(trimmed);
  if (!Number.isSafeInteger(value) || value < 1) return null;
  return value;
}

function validateVolume(
  parsed: number | null | undefined,
  context: ShrinkVolumeContext,
  maximumGiB?: number,
): ShrinkFieldError | undefined {
  if (parsed === null) return "invalid";
  if (parsed === undefined) return undefined;
  if (parsed >= context.currentGiB) return "notSmaller";
  if (maximumGiB !== undefined && parsed > maximumGiB) return "aboveMax";
  if (context.usedBytes !== null && parsed * GIB_BYTES <= context.usedBytes) return "belowUsage";
  return undefined;
}

export function validateStorageShrink(
  homeRaw: string,
  temporaryRaw: string,
  home: ShrinkVolumeContext,
  temporary: ShrinkVolumeContext,
): ShrinkValidation {
  const homeParsed = parseGiBTarget(homeRaw);
  const temporaryParsed = parseGiBTarget(temporaryRaw);
  const homeError = validateVolume(homeParsed, home);
  const temporaryError = validateVolume(temporaryParsed, temporary, TEMPORARY_STORAGE_MAX_GIB);
  const empty = homeParsed === undefined && temporaryParsed === undefined;
  if (homeError || temporaryError || empty) {
    return { homeError, temporaryError, empty, targets: null };
  }
  const targets: { home_gib?: number; temporary_storage_gib?: number } = {};
  if (homeParsed !== undefined && homeParsed !== null) targets.home_gib = homeParsed;
  if (temporaryParsed !== undefined && temporaryParsed !== null) targets.temporary_storage_gib = temporaryParsed;
  return { empty, targets };
}

export function storageShrinkPhaseActive(phase: StorageShrinkPhase): boolean {
  return phase !== "completed" && phase !== "failed" && phase !== "reclaimed";
}

export function storageShrinkInFlight(operation: Pick<StorageShrinkOperation, "phase"> | null): boolean {
  return operation !== null && storageShrinkPhaseActive(operation.phase);
}

export function storageShrinkReclaimable(operation: OperationStatus | null): boolean {
  return operation !== null
    && operation.phase === "completed"
    && operation.reclaimable
    && operation.sources_retained;
}

export interface StorageShrinkResumeOperation {
  phase: StorageShrinkPhase;
  switched: boolean;
  resume: boolean;
  sources_retained: boolean;
  reclaimable: boolean;
  request: Pick<StorageShrinkRequest, "home_gib" | "temporary_storage_gib" | "reclaim_source">;
  home: StorageShrinkVolume;
  temporary: StorageShrinkVolume;
  original_home_gib: number;
  original_temporary_storage_gib: number;
}

export interface StorageShrinkResumeContext {
  state: WorkspaceState;
  resources: { disk_gib: number };
  storage_policy: { temporary_storage_gib: number };
  home_volume_binding?: WorkspaceHomeVolumeBinding | null;
  scratch_volume_binding?: WorkspaceHomeVolumeBinding | null;
}

function bindingMatches(
  binding: WorkspaceHomeVolumeBinding | null | undefined,
  expected: WorkspaceHomeVolumeBinding,
): boolean {
  return binding != null
    && binding.namespace === expected.namespace
    && binding.claim_name === expected.claim_name
    && binding.claim_uid === expected.claim_uid
    && binding.capacity_gib === expected.capacity_gib;
}

function volumeIdentityDistinct(source: StorageVolumeIdentity, destination: StorageVolumeIdentity): boolean {
  const populated = [source, destination].every((identity) =>
    [identity.pv_name, identity.pv_uid, identity.csi_driver, identity.csi_volume_handle, identity.storage_class_name]
      .every((value) => typeof value === "string" && value.trim() !== ""));
  return populated
    && source.csi_driver === destination.csi_driver
    && source.storage_class_name === destination.storage_class_name
    && source.pv_name !== destination.pv_name
    && source.pv_uid !== destination.pv_uid
    && source.csi_volume_handle !== destination.csi_volume_handle;
}

function volumeResumeReady(
  volume: StorageShrinkVolume,
  requestTarget: number | undefined,
  currentBinding: WorkspaceHomeVolumeBinding | null | undefined,
  currentGiB: number,
  originalGiB: number,
): boolean {
  if (volume.target_gib === null) {
    return requestTarget === undefined
      && volume.destination === null
      && volume.source !== null
      && volume.source.capacity_gib === currentGiB
      && volume.source.capacity_gib === originalGiB
      && bindingMatches(currentBinding, volume.source);
  }
  const { source, destination, source_volume, destination_volume } = volume;
  return requestTarget === volume.target_gib
    && volume.target_gib > 0
    && volume.target_gib < originalGiB
    && volume.verified
    && volume.used_bytes !== null
    && Number.isSafeInteger(volume.used_bytes)
    && volume.used_bytes >= 0
    && volume.used_bytes < volume.target_gib * GIB_BYTES
    && volume.used_inodes !== null
    && Number.isSafeInteger(volume.used_inodes)
    && volume.used_inodes >= 0
    && source !== null
    && destination !== null
    && source.capacity_gib === originalGiB
    && source.namespace === destination.namespace
    && source.claim_name !== destination.claim_name
    && destination.capacity_gib === volume.target_gib
    && destination.capacity_gib === currentGiB
    && source.claim_uid !== destination.claim_uid
    && source_volume !== null
    && destination_volume !== null
    && volumeIdentityDistinct(source_volume, destination_volume)
    && bindingMatches(currentBinding, destination);
}

export function storageShrinkResumable(
  operation: StorageShrinkResumeOperation | null,
  workspace: StorageShrinkResumeContext | null | undefined,
): boolean {
  if (operation === null || workspace == null) return false;
  if (workspace.state !== "ready") return false;
  if (operation.phase !== "failed" || !operation.switched || !operation.resume) return false;
  if (!operation.sources_retained || operation.reclaimable || operation.request.reclaim_source === true) return false;
  if (operation.home.target_gib === null && operation.temporary.target_gib === null) return false;
  return volumeResumeReady(
    operation.home,
    operation.request.home_gib ?? undefined,
    workspace.home_volume_binding,
    workspace.resources.disk_gib,
    operation.original_home_gib,
  ) && volumeResumeReady(
    operation.temporary,
    operation.request.temporary_storage_gib ?? undefined,
    workspace.scratch_volume_binding,
    workspace.storage_policy.temporary_storage_gib,
    operation.original_temporary_storage_gib,
  );
}

const phaseLabelKeys: Record<StorageShrinkPhase, MessageKey> = {
  queued: "shrinkPhaseQueued",
  stopping: "shrinkPhaseStopping",
  measuring: "shrinkPhaseMeasuring",
  copying: "shrinkPhaseCopying",
  verifying: "shrinkPhaseVerifying",
  switching: "shrinkPhaseSwitching",
  resuming: "shrinkPhaseResuming",
  rolling_back: "shrinkPhaseRollingBack",
  completed: "shrinkPhaseCompleted",
  failed: "shrinkPhaseFailed",
  reclaiming: "shrinkPhaseReclaiming",
  reclaimed: "shrinkPhaseReclaimed",
};

export function storageShrinkPhaseLabelKey(phase: StorageShrinkPhase): MessageKey {
  return phaseLabelKeys[phase] ?? "shrinkPhaseQueued";
}

const fieldErrorKeys: Record<ShrinkFieldError, MessageKey> = {
  invalid: "shrinkErrorInvalid",
  notSmaller: "shrinkErrorNotSmaller",
  aboveMax: "shrinkErrorAboveMax",
  belowUsage: "shrinkErrorBelowUsage",
};

export function shrinkFieldErrorKey(error: ShrinkFieldError): MessageKey {
  return fieldErrorKeys[error];
}
