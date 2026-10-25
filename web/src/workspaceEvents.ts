import type { MessageKey } from "./i18n";
import type { SchedulingBlocker, WorkspaceRuntimeEvent } from "./types";

export interface EventPresentationKeys {
  label: MessageKey;
  description: MessageKey;
}

const categoryKeys: Record<WorkspaceRuntimeEvent["category"], EventPresentationKeys> = {
  disk_pressure: { label: "eventCategoryDiskPressure", description: "eventCategoryDiskPressureDescription" },
  evicted: { label: "eventCategoryEvicted", description: "eventCategoryEvictedDescription" },
  temporary_storage_provisioning: { label: "eventCategoryTemporaryStorageProvisioning", description: "eventCategoryTemporaryStorageProvisioningDescription" },
  temporary_storage_attachment: { label: "eventCategoryTemporaryStorageAttachment", description: "eventCategoryTemporaryStorageAttachmentDescription" },
  volume_unavailable: { label: "eventCategoryVolumeUnavailable", description: "eventCategoryVolumeUnavailableDescription" },
  ephemeral_storage: { label: "eventCategoryEphemeralStorage", description: "eventCategoryEphemeralStorageDescription" },
  memory_pressure: { label: "eventCategoryMemoryPressure", description: "eventCategoryMemoryPressureDescription" },
  pid_pressure: { label: "eventCategoryPidPressure", description: "eventCategoryPidPressureDescription" },
  container_backoff: { label: "eventCategoryContainerBackoff", description: "eventCategoryContainerBackoffDescription" },
  image_pull: { label: "eventCategoryImagePull", description: "eventCategoryImagePullDescription" },
  scheduling: { label: "eventCategoryScheduling", description: "eventCategorySchedulingDescription" },
  health_check: { label: "eventCategoryHealthCheck", description: "eventCategoryHealthCheckDescription" },
  other: { label: "eventCategoryOther", description: "eventCategoryOtherDescription" },
};

/**
 * Recognized Kubernetes warning reasons that otherwise surface under the
 * generic "other" category. Keys must stay aligned with the backend's safe
 * reason allowlist; both FailedCreatePodSandBox spellings are accepted.
 */
const otherReasonKeys: Record<string, EventPresentationKeys> = {
  FailedSync: { label: "eventReasonFailedSync", description: "eventReasonFailedSyncDescription" },
  NodeNotReady: { label: "eventReasonNodeNotReady", description: "eventReasonNodeNotReadyDescription" },
  FailedPostStartHook: { label: "eventReasonFailedPostStartHook", description: "eventReasonFailedPostStartHookDescription" },
  FailedCreatePodSandBox: { label: "eventReasonFailedCreatePodSandbox", description: "eventReasonFailedCreatePodSandboxDescription" },
  FailedCreatePodSandbox: { label: "eventReasonFailedCreatePodSandbox", description: "eventReasonFailedCreatePodSandboxDescription" },
  FailedCreate: { label: "eventReasonFailedCreate", description: "eventReasonFailedCreateDescription" },
  FailedBinding: { label: "eventReasonFailedBinding", description: "eventReasonFailedBindingDescription" },
  DeadlineExceeded: { label: "eventReasonDeadlineExceeded", description: "eventReasonDeadlineExceededDescription" },
  ExceededGracePeriod: { label: "eventReasonExceededGracePeriod", description: "eventReasonExceededGracePeriodDescription" },
};

/**
 * Recognized Kubernetes scheduler blocking causes carried on scheduling
 * events. These are capacity/constraint failures, not live utilization, so
 * the copy never promises that startup resumes on its own. Keys must stay
 * aligned with the backend's SchedulingBlocker enum (snake_case).
 */
const schedulingBlockerKeys: Record<SchedulingBlocker, EventPresentationKeys> = {
  cpu_capacity: { label: "eventSchedulingBlockerCpuCapacity", description: "eventSchedulingBlockerCpuCapacityDescription" },
  memory_capacity: { label: "eventSchedulingBlockerMemoryCapacity", description: "eventSchedulingBlockerMemoryCapacityDescription" },
  storage_capacity: { label: "eventSchedulingBlockerStorageCapacity", description: "eventSchedulingBlockerStorageCapacityDescription" },
  node_affinity: { label: "eventSchedulingBlockerNodeAffinity", description: "eventSchedulingBlockerNodeAffinityDescription" },
  volume_affinity: { label: "eventSchedulingBlockerVolumeAffinity", description: "eventSchedulingBlockerVolumeAffinityDescription" },
  taint: { label: "eventSchedulingBlockerTaint", description: "eventSchedulingBlockerTaintDescription" },
  node_unavailable: { label: "eventSchedulingBlockerNodeUnavailable", description: "eventSchedulingBlockerNodeUnavailableDescription" },
};

/** Maps a recognized scheduler blocker to title/description keys, or null. */
export function schedulingBlockerPresentationKeys(blocker: string | null | undefined): EventPresentationKeys | null {
  if (!blocker) return null;
  // Own-property lookup only: runtime blocker strings are arbitrary, and a
  // plain index would happily return inherited properties like "__proto__"
  // or "toString" as if they were recognized blockers.
  return Object.hasOwn(schedulingBlockerKeys, blocker) ? schedulingBlockerKeys[blocker as SchedulingBlocker] : null;
}

/**
 * Returns one title/description entry per recognized scheduler blocker on a
 * scheduling event, deduplicated while preserving backend order. Unknown,
 * malformed, or missing blocker data yields an empty list, and non-scheduling
 * categories never surface stale blockers; callers fall back to the generic
 * eventPresentationKeys copy in all of those cases.
 */
export function eventSchedulingPresentationKeys(event: Pick<WorkspaceRuntimeEvent, "category" | "details">): EventPresentationKeys[] {
  if (event.category !== "scheduling") return [];
  const raw = event.details.scheduling_blockers;
  if (!Array.isArray(raw)) return [];
  const seen = new Set<string>();
  const entries: EventPresentationKeys[] = [];
  for (const value of raw) {
    if (typeof value !== "string" || seen.has(value)) continue;
    const keys = schedulingBlockerPresentationKeys(value);
    if (!keys) continue;
    seen.add(value);
    entries.push(keys);
  }
  return entries;
}

/** Maps a recognized Kubernetes warning reason to title/description keys, or null. */
export function eventReasonKeys(reason: string | null | undefined): EventPresentationKeys | null {
  if (!reason) return null;
  // Own-property lookup only: runtime reason strings are arbitrary, and a
  // plain index would happily return inherited properties like "__proto__"
  // or "toString" as if they were recognized reasons.
  return Object.hasOwn(otherReasonKeys, reason) ? otherReasonKeys[reason] : null;
}

/**
 * Resolves the localized title/description for an event. Reason-specific
 * explanations only apply to the generic "other" category; recognized
 * categories keep their category-specific copy. Historical events with an
 * empty details object (no reason, blockers, or diagnostic) honestly keep
 * the generic category copy.
 */
export function eventPresentationKeys(event: Pick<WorkspaceRuntimeEvent, "category" | "details">): EventPresentationKeys {
  if (event.category === "other") {
    const byReason = eventReasonKeys(event.details.reason);
    if (byReason) return byReason;
  }
  // The category type is strict, but runtime data can still carry an unknown
  // value; fall back to the generic "other" copy in that case (and keep the
  // lookup own-property-only for the same reason-string hazard).
  return Object.hasOwn(categoryKeys, event.category) ? categoryKeys[event.category] : categoryKeys.other;
}
