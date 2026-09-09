import assert from "node:assert/strict";
import test from "node:test";
import { formatBytesAsGiB, formatCores, usageBarPercentage, usagePercentage } from "./usageSummaryPresentation.ts";

test("small nonzero CPU usage is not displayed as idle", () => {
  assert.equal(formatCores(22), "0.022");
  assert.equal(formatCores(1), "0.001");
  assert.equal(formatCores(1_025), "1.025");
  assert.equal(formatCores(6_000), "6");
  assert.equal(formatCores(0), "0");
});

test("actual usage percentage compares runtime observation with requested capacity", () => {
  assert.equal(usagePercentage(1_500, 1_000), 150);
  assert.equal(usageBarPercentage(usagePercentage(1_500, 1_000)), 100);
  assert.equal(usagePercentage(250, 1_000), 25);
  assert.equal(usagePercentage(22, 27_000), 0.08);
  assert.equal(usagePercentage(null, 1_000), null);
  assert.equal(usagePercentage(0, 0), null);
});

test("disk observations are displayed from bytes without treating null as zero", () => {
  assert.equal(formatBytesAsGiB(3 * 1024 ** 3), "3");
  assert.equal(formatBytesAsGiB(1536 * 1024 ** 2), "1.5");
});
