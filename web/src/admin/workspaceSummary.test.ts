import assert from "node:assert/strict";
import test from "node:test";

import { workspaceStateCounts } from "./workspaceSummary.ts";

test("operations status uses the server summary across all pages", () => {
  const counts = workspaceStateCounts({
    total_count: 12_004,
    requested: { cpu_millis: 48_000, memory_mib: 96_000, gpu_count: 2, disk_gib: 240_000 },
    state_counts: { ready: 11_990, stopped: 12, failed: 2 },
  });
  assert.deepEqual(counts, { ready: 11_990, stopped: 12, failed: 2 });
});

test("operations status is empty until the server summary arrives", () => {
  assert.deepEqual(workspaceStateCounts(null), {});
});
