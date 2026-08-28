import assert from "node:assert/strict";
import test from "node:test";

import {
  aggregateRuntimeUsage,
  formatCpuMillis,
  formatMemoryMiB,
  parseCpuMillis,
  parseMemoryMiB,
  usagePercent,
} from "./workspaceMetrics.ts";
import type { WorkspaceRuntime } from "./types.ts";

test("parses Kubernetes CPU quantities into millicores", () => {
  assert.ok(Math.abs((parseCpuMillis("2482467n") ?? 0) - 2.482467) < 0.0000001);
  assert.ok(Math.abs((parseCpuMillis("1152u") ?? 0) - 1.152) < 0.0000001);
  assert.equal(parseCpuMillis("350m"), 350);
  assert.equal(parseCpuMillis("1.5"), 1500);
  assert.equal(parseCpuMillis("100µ"), 0.1);
  assert.equal(parseCpuMillis("1e-3"), 1);
  assert.equal(parseCpuMillis("bad"), null);
});

test("parses Kubernetes memory quantities into MiB", () => {
  assert.equal(parseMemoryMiB("51200Ki"), 50);
  assert.equal(parseMemoryMiB("1.5Gi"), 1536);
  assert.equal(parseMemoryMiB("1048576"), 1);
  assert.equal(parseMemoryMiB("bad"), null);
});

test("uses only complete workspace-container usage and clamps visual percentages", () => {
  const runtime = {
    metrics: [
      { pod: "workspace-0", container: "workspace", cpu: "2482467n", memory: "51196Ki" },
      { pod: "workspace-0", container: "ttyd", cpu: "1152n", memory: "16828Ki" },
      { pod: "workspace-0", container: "buildkitd", cpu: "3248801n", memory: "51260Ki" },
    ],
  } as WorkspaceRuntime;
  const usage = aggregateRuntimeUsage(runtime);
  assert.ok(usage.cpuMillis !== null && Math.abs(usage.cpuMillis - 2.482467) < 0.00001);
  assert.ok(usage.memoryMiB !== null && Math.abs(usage.memoryMiB - 49.99609375) < 0.00001);
  assert.ok(Math.abs((usagePercent(usage.cpuMillis, 6000) ?? 0) - usage.cpuMillis / 60) < 0.0000001);
  assert.equal(usagePercent(9000, 6000), 100);
  assert.equal(usagePercent(null, 6000), null);
  assert.equal(formatCpuMillis(usage.cpuMillis), "2.5m");
  assert.equal(formatMemoryMiB(usage.memoryMiB), "50 MiB");
});

test("does not show a partial workspace total when a metric cannot be parsed", () => {
  const runtime = {
    metrics: [
      { pod: "workspace-0", container: "workspace", cpu: "1m", memory: "1Mi" },
      { pod: "workspace-1", container: "workspace", cpu: null, memory: "2Mi" },
      { pod: "workspace-0", container: "ttyd", cpu: "9m", memory: "9Mi" },
    ],
  } as WorkspaceRuntime;
  assert.deepEqual(aggregateRuntimeUsage(runtime), { cpuMillis: null, memoryMiB: 3 });
  assert.deepEqual(aggregateRuntimeUsage({ metrics: [] } as unknown as WorkspaceRuntime), { cpuMillis: null, memoryMiB: null });
});
