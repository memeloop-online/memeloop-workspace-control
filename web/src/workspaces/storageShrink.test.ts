import assert from "node:assert/strict";
import test from "node:test";
import {
  TEMPORARY_STORAGE_MAX_GIB,
  parseGiBTarget,
  storageShrinkInFlight,
  storageShrinkPhaseActive,
  storageShrinkReclaimable,
  validateStorageShrink,
} from "./storageShrink.ts";

const GIB = 1024 ** 3;
const home = { currentGiB: 100, usedBytes: 40 * GIB };
const temporary = { currentGiB: 50, usedBytes: null };

test("target fields accept whole GiB numbers only", () => {
  assert.equal(parseGiBTarget(""), undefined);
  assert.equal(parseGiBTarget("   "), undefined);
  assert.equal(parseGiBTarget("20"), 20);
  assert.equal(parseGiBTarget(" 20 "), 20);
  assert.equal(parseGiBTarget("0"), null);
  assert.equal(parseGiBTarget("-5"), null);
  assert.equal(parseGiBTarget("1.5"), null);
  assert.equal(parseGiBTarget("20.0"), null);
  assert.equal(parseGiBTarget("abc"), null);
});

test("both volumes can shrink together and a single volume is allowed", () => {
  assert.deepEqual(validateStorageShrink("80", "40", home, temporary).targets, {
    home_gib: 80,
    temporary_storage_gib: 40,
  });
  assert.deepEqual(validateStorageShrink("80", "", home, temporary).targets, { home_gib: 80 });
  assert.deepEqual(validateStorageShrink("", "40", home, temporary).targets, { temporary_storage_gib: 40 });
});

test("at least one target is required", () => {
  const result = validateStorageShrink("", "", home, temporary);
  assert.equal(result.empty, true);
  assert.equal(result.targets, null);
});

test("targets must stay below the configured size", () => {
  assert.equal(validateStorageShrink("100", "", home, temporary).homeError, "notSmaller");
  assert.equal(validateStorageShrink("150", "", home, temporary).homeError, "notSmaller");
  assert.equal(validateStorageShrink("", "50", home, temporary).temporaryError, "notSmaller");
  assert.equal(validateStorageShrink("99", "", home, temporary).homeError, undefined);
});

test("temporary targets respect the backend maximum", () => {
  const largeTemporary = { currentGiB: 4_096, usedBytes: null };
  assert.equal(validateStorageShrink("", String(TEMPORARY_STORAGE_MAX_GIB), home, largeTemporary).temporaryError, undefined);
  assert.equal(validateStorageShrink("", String(TEMPORARY_STORAGE_MAX_GIB + 1), home, largeTemporary).temporaryError, "aboveMax");
});

test("known usage blocks targets that would not fit the data", () => {
  assert.equal(validateStorageShrink("40", "", home, temporary).homeError, "belowUsage");
  assert.equal(validateStorageShrink("41", "", home, temporary).homeError, undefined);
  const partialGiB = { currentGiB: 100, usedBytes: 40 * GIB + 1 };
  assert.equal(validateStorageShrink("41", "", partialGiB, temporary).homeError, undefined);
  assert.equal(validateStorageShrink("40", "", partialGiB, temporary).homeError, "belowUsage");
});

test("unknown usage never blocks and never counts as zero", () => {
  const unknown = { currentGiB: 100, usedBytes: null };
  const result = validateStorageShrink("1", "", unknown, temporary);
  assert.equal(result.homeError, undefined);
  assert.deepEqual(result.targets, { home_gib: 1 });
});

test("any field error holds back the whole request", () => {
  const result = validateStorageShrink("100", "40", home, temporary);
  assert.equal(result.targets, null);
  assert.equal(result.homeError, "notSmaller");
  assert.equal(result.temporaryError, undefined);
  const invalid = validateStorageShrink("abc", "40", home, temporary);
  assert.equal(invalid.homeError, "invalid");
  assert.equal(invalid.temporaryError, undefined);
  assert.equal(invalid.targets, null);
});

test("phases report active until completed or failed", () => {
  for (const phase of ["queued", "stopping", "measuring", "copying", "verifying", "switching", "resuming", "rolling_back", "reclaiming"] as const) {
    assert.equal(storageShrinkPhaseActive(phase), true, phase);
  }
  assert.equal(storageShrinkPhaseActive("completed"), false);
  assert.equal(storageShrinkPhaseActive("failed"), false);
  assert.equal(storageShrinkPhaseActive("reclaimed"), false);
  assert.equal(storageShrinkInFlight(null), false);
  assert.equal(storageShrinkInFlight({ phase: "copying" }), true);
  assert.equal(storageShrinkInFlight({ phase: "failed" }), false);
});

test("reclaim requires a completed operation that still retains reclaimable sources", () => {
  assert.equal(storageShrinkReclaimable(null), false);
  assert.equal(storageShrinkReclaimable({ phase: "completed", reclaimable: true, sources_retained: true }), true);
  assert.equal(storageShrinkReclaimable({ phase: "completed", reclaimable: true, sources_retained: false }), false);
  assert.equal(storageShrinkReclaimable({ phase: "completed", reclaimable: false, sources_retained: true }), false);
  assert.equal(storageShrinkReclaimable({ phase: "copying", reclaimable: true, sources_retained: true }), false);
  assert.equal(storageShrinkReclaimable({ phase: "reclaimed", reclaimable: true, sources_retained: false }), false);
});
