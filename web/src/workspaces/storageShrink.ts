import type { MessageKey } from "../i18n";
import type { StorageShrinkOperation, StorageShrinkPhase } from "../types";

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
