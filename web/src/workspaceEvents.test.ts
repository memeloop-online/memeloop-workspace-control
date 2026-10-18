import assert from "node:assert/strict";
import test from "node:test";

import { eventPresentationKeys, eventReasonKeys, eventSchedulingPresentationKeys, schedulingBlockerPresentationKeys } from "./workspaceEvents.ts";

test("recognized warning reasons under the other category get specific keys", () => {
  assert.deepEqual(eventReasonKeys("FailedSync"), { label: "eventReasonFailedSync", description: "eventReasonFailedSyncDescription" });
  assert.deepEqual(eventReasonKeys("NodeNotReady"), { label: "eventReasonNodeNotReady", description: "eventReasonNodeNotReadyDescription" });
  assert.deepEqual(eventReasonKeys("FailedPostStartHook"), { label: "eventReasonFailedPostStartHook", description: "eventReasonFailedPostStartHookDescription" });
  assert.deepEqual(eventReasonKeys("FailedCreate"), { label: "eventReasonFailedCreate", description: "eventReasonFailedCreateDescription" });
  assert.deepEqual(eventReasonKeys("FailedBinding"), { label: "eventReasonFailedBinding", description: "eventReasonFailedBindingDescription" });
  assert.deepEqual(eventReasonKeys("DeadlineExceeded"), { label: "eventReasonDeadlineExceeded", description: "eventReasonDeadlineExceededDescription" });
  assert.deepEqual(eventReasonKeys("ExceededGracePeriod"), { label: "eventReasonExceededGracePeriod", description: "eventReasonExceededGracePeriodDescription" });
});

test("both pod sandbox spellings map to the same explanation", () => {
  const expected = { label: "eventReasonFailedCreatePodSandbox", description: "eventReasonFailedCreatePodSandboxDescription" };
  assert.deepEqual(eventReasonKeys("FailedCreatePodSandBox"), expected);
  assert.deepEqual(eventReasonKeys("FailedCreatePodSandbox"), expected);
  assert.deepEqual(eventPresentationKeys({ category: "other", reason: "FailedCreatePodSandBox" }), expected);
  assert.deepEqual(eventPresentationKeys({ category: "other", reason: "FailedCreatePodSandbox" }), expected);
});

test("other events with a recognized reason use the reason-specific keys", () => {
  assert.deepEqual(
    eventPresentationKeys({ category: "other", reason: "NodeNotReady" }),
    { label: "eventReasonNodeNotReady", description: "eventReasonNodeNotReadyDescription" },
  );
});

test("missing or unknown other reasons fall back to the honest generic copy", () => {
  const fallback = { label: "eventCategoryOther", description: "eventCategoryOtherDescription" };
  assert.equal(eventReasonKeys(null), null);
  assert.equal(eventReasonKeys(undefined), null);
  assert.equal(eventReasonKeys(""), null);
  assert.equal(eventReasonKeys("SomethingUnexpected"), null);
  assert.deepEqual(eventPresentationKeys({ category: "other", reason: null }), fallback);
  assert.deepEqual(eventPresentationKeys({ category: "other" }), fallback);
  assert.deepEqual(eventPresentationKeys({ category: "other", reason: "SomethingUnexpected" }), fallback);
});

test("prototype property names are not treated as recognized reasons", () => {
  const fallback = { label: "eventCategoryOther", description: "eventCategoryOtherDescription" };
  for (const reason of ["__proto__", "constructor", "toString", "hasOwnProperty", "valueOf"]) {
    assert.equal(eventReasonKeys(reason), null, `reason ${reason} must not resolve to inherited properties`);
    assert.deepEqual(eventPresentationKeys({ category: "other", reason }), fallback);
  }
});

test("malformed runtime categories fall back to the generic other copy", () => {
  const fallback = { label: "eventCategoryOther", description: "eventCategoryOtherDescription" };
  // Runtime payloads can carry values outside the strict category union;
  // casts here only exercise that defensive path.
  for (const category of ["something_new", "__proto__", "constructor", "toString"]) {
    const event = { category, reason: "NodeNotReady" } as unknown as Parameters<typeof eventPresentationKeys>[0];
    assert.deepEqual(eventPresentationKeys(event), fallback, `category ${category} must fall back`);
  }
});

test("recognized categories keep their category-specific copy even with a known reason", () => {
  assert.deepEqual(
    eventPresentationKeys({ category: "scheduling", reason: "FailedBinding" }),
    { label: "eventCategoryScheduling", description: "eventCategorySchedulingDescription" },
  );
  assert.deepEqual(
    eventPresentationKeys({ category: "temporary_storage_provisioning", reason: "FailedCreate" }),
    { label: "eventCategoryTemporaryStorageProvisioning", description: "eventCategoryTemporaryStorageProvisioningDescription" },
  );
  assert.deepEqual(
    eventPresentationKeys({ category: "container_backoff", reason: null }),
    { label: "eventCategoryContainerBackoff", description: "eventCategoryContainerBackoffDescription" },
  );
});

test("scheduler capacity causes distinguish CPU, memory and CSI storage", () => {
  assert.deepEqual(eventSchedulingPresentationKeys({ category: "scheduling", scheduling_blockers: ["cpu_capacity"] }), [
    { label: "eventSchedulingBlockerCpuCapacity", description: "eventSchedulingBlockerCpuCapacityDescription" },
  ]);
  assert.deepEqual(eventSchedulingPresentationKeys({ category: "scheduling", scheduling_blockers: ["memory_capacity", "storage_capacity"] }), [
    { label: "eventSchedulingBlockerMemoryCapacity", description: "eventSchedulingBlockerMemoryCapacityDescription" },
    { label: "eventSchedulingBlockerStorageCapacity", description: "eventSchedulingBlockerStorageCapacityDescription" },
  ]);
});

test("mixed scheduler constraints retain order, dedupe causes and ignore unknown entries", () => {
  const event = { category: "scheduling", scheduling_blockers: ["taint", "new_cause", "cpu_capacity", "taint", "volume_affinity", "node_affinity", "node_unavailable"] } as unknown as Parameters<typeof eventSchedulingPresentationKeys>[0];
  assert.deepEqual(eventSchedulingPresentationKeys(event).map((entry) => entry.label), [
    "eventSchedulingBlockerTaint", "eventSchedulingBlockerCpuCapacity", "eventSchedulingBlockerVolumeAffinity", "eventSchedulingBlockerNodeAffinity", "eventSchedulingBlockerNodeUnavailable",
  ]);
});

test("old and malformed blocker fields preserve the generic scheduling fallback", () => {
  const fallback = { label: "eventCategoryScheduling", description: "eventCategorySchedulingDescription" };
  for (const scheduling_blockers of [undefined, null, [], "cpu_capacity", {}, [null, 42, {}, "__proto__", "constructor", "toString", "unknown"]]) {
    const event = { category: "scheduling", reason: "FailedScheduling", scheduling_blockers } as unknown as Parameters<typeof eventSchedulingPresentationKeys>[0];
    assert.deepEqual(eventSchedulingPresentationKeys(event), []);
    assert.deepEqual(eventPresentationKeys(event), fallback);
  }
  for (const blocker of [null, undefined, "", "__proto__", "constructor", "toString", "new_cause"]) {
    assert.equal(schedulingBlockerPresentationKeys(blocker), null);
  }
});

test("non-scheduling events never surface stale scheduling causes", () => {
  const event = { category: "volume_unavailable", scheduling_blockers: ["cpu_capacity"] } as const;
  assert.deepEqual(eventSchedulingPresentationKeys({ ...event, scheduling_blockers: [...event.scheduling_blockers] }), []);
  assert.deepEqual(eventPresentationKeys(event), {
    label: "eventCategoryVolumeUnavailable", description: "eventCategoryVolumeUnavailableDescription",
  });
});
