import assert from "node:assert/strict";
import test from "node:test";

import {
  RUNTIME_PROFILES,
  isHighRiskRuntimeProfile,
  runtimeProfileLabel,
} from "./runtimeProfiles.ts";

test("runtime profiles expose the complete controlled set", () => {
  assert.deepEqual(
    RUNTIME_PROFILES.map((profile) => profile.value),
    [
      "standard",
      "rust_dev",
      "node_dev",
      "maintainance",
    ],
  );
});

test("only the maintainance profile is marked high risk", () => {
  for (const profile of RUNTIME_PROFILES) {
    assert.equal(
      isHighRiskRuntimeProfile(profile.value),
      profile.value === "maintainance",
    );
  }
  assert.equal(runtimeProfileLabel("maintainance"), "Maintainance");
});
