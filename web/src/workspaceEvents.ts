import type { MessageKey } from "./i18n";
import type { WorkspaceRuntimeEvent } from "./types";

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
 * categories keep their category-specific copy.
 */
export function eventPresentationKeys(event: Pick<WorkspaceRuntimeEvent, "category" | "reason">): EventPresentationKeys {
  if (event.category === "other") {
    const byReason = eventReasonKeys(event.reason);
    if (byReason) return byReason;
  }
  // The category type is strict, but runtime data can still carry an unknown
  // value; fall back to the generic "other" copy in that case (and keep the
  // lookup own-property-only for the same reason-string hazard).
  return Object.hasOwn(categoryKeys, event.category) ? categoryKeys[event.category] : categoryKeys.other;
}
