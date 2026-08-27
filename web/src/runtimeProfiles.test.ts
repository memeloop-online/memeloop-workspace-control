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
      "coder_rust_dev",
      "coder_node_dev",
      "coder_token_center_rust_dev",
      "coder_cluster_admin",
    ],
  );
});

test("only the cluster administrator profile is marked high risk", () => {
  for (const profile of RUNTIME_PROFILES) {
    assert.equal(
      isHighRiskRuntimeProfile(profile.value),
      profile.value === "coder_cluster_admin",
    );
  }
  assert.equal(runtimeProfileLabel("coder_cluster_admin"), "Coder 集群管理员");
});
