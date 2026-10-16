import assert from "node:assert/strict";
import test from "node:test";

import { eventPresentationKeys, eventReasonKeys } from "./workspaceEvents.ts";

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
