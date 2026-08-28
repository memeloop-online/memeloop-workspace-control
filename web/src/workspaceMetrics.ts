import type { WorkspaceRuntime } from "./types";

const CPU_FACTORS: Record<string, number> = {
  n: 0.000001,
  u: 0.001,
  µ: 0.001,
  m: 1,
  "": 1000,
};

const MEMORY_FACTORS_MIB: Record<string, number> = {
  Ki: 1 / 1024,
  Mi: 1,
  Gi: 1024,
  Ti: 1024 ** 2,
  Pi: 1024 ** 3,
  Ei: 1024 ** 4,
  K: 1000 / 1024 ** 2,
  M: 1000 ** 2 / 1024 ** 2,
  G: 1000 ** 3 / 1024 ** 2,
  T: 1000 ** 4 / 1024 ** 2,
  P: 1000 ** 5 / 1024 ** 2,
  E: 1000 ** 6 / 1024 ** 2,
  "": 1 / 1024 ** 2,
};

export interface RuntimeUsage {
  cpuMillis: number | null;
  memoryMiB: number | null;
}

export function parseCpuMillis(quantity: string | null): number | null {
  if (!quantity) return null;
  const match = quantity.trim().match(/^((?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eE][+-]?[0-9]+)?)(n|u|µ|m)?$/);
  if (!match) return null;
  const value = Number(match[1]);
  const factor = CPU_FACTORS[match[2] ?? ""];
  return Number.isFinite(value) && factor !== undefined ? value * factor : null;
}

export function parseMemoryMiB(quantity: string | null): number | null {
  if (!quantity) return null;
  const match = quantity.trim().match(/^((?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eE][+-]?[0-9]+)?)(Ki|Mi|Gi|Ti|Pi|Ei|K|M|G|T|P|E)?$/);
  if (!match) return null;
  const value = Number(match[1]);
  const factor = MEMORY_FACTORS_MIB[match[2] ?? ""];
  return Number.isFinite(value) && factor !== undefined ? value * factor : null;
}

export function aggregateRuntimeUsage(runtime: WorkspaceRuntime): RuntimeUsage {
  const workspaceMetrics = runtime.metrics.filter((metric) => metric.container === "workspace");
  if (workspaceMetrics.length === 0) return { cpuMillis: null, memoryMiB: null };
  let cpuMillis = 0;
  let memoryMiB = 0;
  let cpuComplete = true;
  let memoryComplete = true;
  for (const metric of workspaceMetrics) {
    const cpu = parseCpuMillis(metric.cpu);
    const memory = parseMemoryMiB(metric.memory);
    if (cpu === null) cpuComplete = false;
    else cpuMillis += cpu;
    if (memory === null) memoryComplete = false;
    else memoryMiB += memory;
  }
  return {
    cpuMillis: cpuComplete ? cpuMillis : null,
    memoryMiB: memoryComplete ? memoryMiB : null,
  };
}

export function usagePercent(actual: number | null, requested: number): number | null {
  if (actual === null || requested <= 0) return null;
  return Math.min(100, Math.max(0, (actual / requested) * 100));
}

export function formatCpuMillis(value: number | null): string {
  if (value === null) return "—";
  if (value > 0 && value < 0.1) return "<0.1m";
  return `${formatNumber(value)}m`;
}

export function formatMemoryMiB(value: number | null): string {
  if (value === null) return "—";
  if (value >= 1024) return `${formatNumber(value / 1024)} GiB`;
  if (value > 0 && value < 1) return "<1 MiB";
  return `${formatNumber(value)} MiB`;
}

export function formatPercent(value: number | null): string {
  if (value === null) return "—";
  if (value > 0 && value < 0.1) return "<0.1%";
  return `${formatNumber(value)}%`;
}

function formatNumber(value: number): string {
  return value.toLocaleString(undefined, { maximumFractionDigits: value < 10 ? 1 : 0 });
}
