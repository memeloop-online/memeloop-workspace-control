import assert from "node:assert/strict";
import test from "node:test";
import {
  TEMPORARY_STORAGE_MAX_GIB,
  parseGiBTarget,
  storageShrinkInFlight,
  storageShrinkPhaseActive,
  storageShrinkReclaimable,
  storageShrinkResumable,
  validateStorageShrink,
} from "./storageShrink.ts";
import type { StorageShrinkResumeContext, StorageShrinkResumeOperation } from "./storageShrink.ts";

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

const homeSource = { namespace: "workspaces", claim_name: "home-old", claim_uid: "home-src", capacity_gib: 100 };
const homeDestination = { namespace: "workspaces", claim_name: "home-new", claim_uid: "home-dst", capacity_gib: 80 };
const temporarySource = { namespace: "workspaces", claim_name: "scratch-old", claim_uid: "tmp-src", capacity_gib: 60 };
const temporaryDestination = { namespace: "workspaces", claim_name: "scratch-new", claim_uid: "tmp-dst", capacity_gib: 40 };

const homeSourceVolume = {
  pv_name: "pv-home-old",
  pv_uid: "pv-src",
  csi_driver: "driver.longhorn.io",
  csi_volume_handle: "vol-src",
  storage_class_name: "longhorn",
  longhorn_namespace: "longhorn-system",
  longhorn_volume_uid: "lh-src",
};
const homeDestinationVolume = {
  ...homeSourceVolume,
  pv_name: "pv-home-new",
  pv_uid: "pv-dst",
  csi_volume_handle: "vol-dst",
  longhorn_volume_uid: "lh-dst",
};

const readyWorkspace: StorageShrinkResumeContext = {
  state: "ready",
  resources: { disk_gib: 80 },
  storage_policy: { temporary_storage_gib: 60 },
  home_volume_binding: homeDestination,
  scratch_volume_binding: temporarySource,
};

function switchedFailure(overrides: Partial<StorageShrinkResumeOperation> = {}): StorageShrinkResumeOperation {
  return {
    phase: "failed",
    switched: true,
    resume: true,
    sources_retained: true,
    reclaimable: false,
    request: { home_gib: 80, reclaim_source: false },
    home: {
      target_gib: 80,
      verified: true,
      source: homeSource,
      destination: homeDestination,
      used_bytes: 40 * GIB,
      used_inodes: 512,
      source_volume: homeSourceVolume,
      destination_volume: homeDestinationVolume,
    },
    temporary: {
      target_gib: null,
      verified: false,
      source: temporarySource,
      destination: null,
      used_bytes: null,
      used_inodes: null,
      source_volume: null,
      destination_volume: null,
    },
    original_home_gib: 100,
    original_temporary_storage_gib: 60,
    ...overrides,
  };
}

test("resume is offered only for a verified switched failure on a ready workspace", () => {
  assert.equal(storageShrinkResumable(switchedFailure(), readyWorkspace), true);
  assert.equal(storageShrinkResumable(null, readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(), null), false);
  assert.equal(storageShrinkResumable(switchedFailure(), { ...readyWorkspace, state: "stopped" }), false);
  assert.equal(storageShrinkResumable(switchedFailure(), { ...readyWorkspace, state: "starting" }), false);
});

test("resume guard rejects operations that are not a clean post-switch failure", () => {
  assert.equal(storageShrinkResumable(switchedFailure({ phase: "completed" }), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure({ phase: "copying" }), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure({ switched: false }), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure({ resume: false }), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure({ sources_retained: false }), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure({ reclaimable: true }), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure({ request: { reclaim_source: true } }), readyWorkspace), false);
});

function withHome(home: Partial<StorageShrinkResumeOperation["home"]>): Partial<StorageShrinkResumeOperation> {
  return { home: { ...switchedFailure().home, ...home } };
}

test("resume guard requires every selected volume verified and bound to the current size", () => {
  assert.equal(storageShrinkResumable(switchedFailure(withHome({ verified: false })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({ used_bytes: null })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({ used_inodes: null })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({ source: null })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({ destination: null })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({
    destination: { ...homeDestination, capacity_gib: 100 },
  })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({
    destination: { ...homeDestination, claim_uid: "home-src" },
  })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({ source_volume: null })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({ destination_volume: null })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({
    destination_volume: { ...homeDestinationVolume, pv_uid: "" },
  })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({
    destination_volume: { ...homeDestinationVolume, pv_uid: " \t " },
  })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({
    destination_volume: { ...homeDestinationVolume, csi_driver: "other.csi.io" },
  })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({
    destination_volume: { ...homeDestinationVolume, storage_class_name: "other-class" },
  })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({
    destination_volume: { ...homeDestinationVolume, pv_name: "pv-home-old" },
  })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({
    destination_volume: { ...homeDestinationVolume, pv_uid: "pv-src" },
  })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({
    destination_volume: { ...homeDestinationVolume, csi_volume_handle: "vol-src" },
  })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({ target_gib: 100 })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({
    source: { ...homeSource, capacity_gib: 90 },
  })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({
    destination: { ...homeDestination, namespace: "other" },
  })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({
    destination: { ...homeDestination, claim_name: "home-old" },
  })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({ used_bytes: 80 * GIB })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({ used_bytes: -1 })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({ used_bytes: Number.NaN })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({ used_inodes: -1 })), readyWorkspace), false);
  assert.equal(storageShrinkResumable(switchedFailure(withHome({ used_inodes: Number.MAX_SAFE_INTEGER + 1 })), readyWorkspace), false);
  const unselectedWithDestination = switchedFailure(withHome({
    target_gib: null, verified: false, destination: homeDestination,
    used_bytes: null, used_inodes: null, source_volume: null, destination_volume: null,
  }));
  assert.equal(storageShrinkResumable(unselectedWithDestination, readyWorkspace), false);
  const resizedWorkspace = { ...readyWorkspace, resources: { disk_gib: 100 } };
  assert.equal(storageShrinkResumable(switchedFailure(), resizedWorkspace), false);
  const bothSelectedWorkspace = {
    ...readyWorkspace,
    storage_policy: { temporary_storage_gib: 40 },
    scratch_volume_binding: temporaryDestination,
  };
  const bothSelected = switchedFailure({
    request: { home_gib: 80, temporary_storage_gib: 40, reclaim_source: false },
    temporary: {
      target_gib: 40,
      verified: true,
      source: temporarySource,
      destination: temporaryDestination,
      used_bytes: 10 * GIB,
      used_inodes: 128,
      source_volume: homeSourceVolume,
      destination_volume: homeDestinationVolume,
    },
  });
  assert.equal(storageShrinkResumable(bothSelected, bothSelectedWorkspace), true);
  assert.equal(storageShrinkResumable(bothSelected, readyWorkspace), false);
});

test("resume guard requires the current binding to fully match the expected claim", () => {
  const wrongNamespace = { ...readyWorkspace, home_volume_binding: { ...homeDestination, namespace: "other" } };
  assert.equal(storageShrinkResumable(switchedFailure(), wrongNamespace), false);
  const wrongName = { ...readyWorkspace, home_volume_binding: { ...homeDestination, claim_name: "home-x" } };
  assert.equal(storageShrinkResumable(switchedFailure(), wrongName), false);
  const wrongUid = { ...readyWorkspace, home_volume_binding: { ...homeDestination, claim_uid: "uid-x" } };
  assert.equal(storageShrinkResumable(switchedFailure(), wrongUid), false);
  const wrongCapacity = { ...readyWorkspace, home_volume_binding: { ...homeDestination, capacity_gib: 81 } };
  assert.equal(storageShrinkResumable(switchedFailure(), wrongCapacity), false);
  const workspaceWithoutBindings = {
    state: "ready",
    resources: { disk_gib: 80 },
    storage_policy: { temporary_storage_gib: 60 },
  } satisfies StorageShrinkResumeContext;
  assert.equal(storageShrinkResumable(switchedFailure(), workspaceWithoutBindings), false);
  const nullScratch = { ...readyWorkspace, scratch_volume_binding: null };
  assert.equal(storageShrinkResumable(switchedFailure(), nullScratch), false);
});

test("resume accepts an unselected volume with an explicitly absent current binding", () => {
  const operation = switchedFailure();
  operation.request.temporary_storage_gib = null;
  operation.temporary.source = null;
  const workspace = { ...readyWorkspace, scratch_volume_binding: null };
  assert.equal(storageShrinkResumable(operation, workspace), true);
  assert.equal(storageShrinkResumable(operation, { ...workspace, scratch_volume_binding: undefined }), false);
  assert.equal(storageShrinkResumable(operation, { ...workspace, storage_policy: { temporary_storage_gib: 59 } }), false);
  assert.equal(storageShrinkResumable(operation, readyWorkspace), false);
});

test("resume guard rejects an operation with no selected volume", () => {
  const idleWorkspace = {
    ...readyWorkspace,
    home_volume_binding: homeSource,
    resources: { disk_gib: 100 },
  };
  const noTargets = switchedFailure({
    request: { home_gib: null, temporary_storage_gib: null, reclaim_source: false },
    home: { ...switchedFailure().home, target_gib: null, verified: false, destination: null, used_bytes: null, used_inodes: null, source_volume: null, destination_volume: null },
    original_home_gib: 100,
  });
  assert.equal(storageShrinkResumable(noTargets, idleWorkspace), false);
  assert.equal(storageShrinkResumable(noTargets, readyWorkspace), false);
});

test("resume guard requires request targets to match volume targets exactly", () => {
  const nullTemporaryTarget = switchedFailure({ request: { home_gib: 80, temporary_storage_gib: null, reclaim_source: false } });
  assert.equal(storageShrinkResumable(nullTemporaryTarget, readyWorkspace), true);
  const wrongHomeTarget = switchedFailure({ request: { home_gib: 79, reclaim_source: false } });
  assert.equal(storageShrinkResumable(wrongHomeTarget, readyWorkspace), false);
  const missingHomeTarget = switchedFailure({ request: { reclaim_source: false } });
  assert.equal(storageShrinkResumable(missingHomeTarget, readyWorkspace), false);
  const nullHomeTarget = switchedFailure({ request: { home_gib: null, reclaim_source: false } });
  assert.equal(storageShrinkResumable(nullHomeTarget, readyWorkspace), false);
  const strayTemporaryTarget = switchedFailure({ request: { home_gib: 80, temporary_storage_gib: 40, reclaim_source: false } });
  assert.equal(storageShrinkResumable(strayTemporaryTarget, readyWorkspace), false);
});

test("resume and reclaim stay mutually exclusive", () => {
  const completed = { phase: "completed", reclaimable: true, sources_retained: true } as const;
  assert.equal(storageShrinkReclaimable(completed), true);
  assert.equal(storageShrinkResumable(switchedFailure({ phase: "completed" }), readyWorkspace), false);
  assert.equal(storageShrinkReclaimable({ ...switchedFailure(), reclaimable: true }), false);
});
