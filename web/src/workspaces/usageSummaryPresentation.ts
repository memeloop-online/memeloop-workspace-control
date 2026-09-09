import type { UsageAvailability } from "../types";

export function usagePercentage(actual: number | null, requested: number): number | null {
  if (actual === null || requested <= 0) return null;
  return Math.max(0, Math.round((actual / requested) * 100));
}

export function usageBarPercentage(percentage: number | null): number | null {
  return percentage === null ? null : Math.min(100, percentage);
}

export function availabilityLabel(availability: UsageAvailability): "available" | "unavailable" | "unknown" { return availability; }
export function formatCores(millis: number): string { return String(Number((millis / 1000).toFixed(3))); }
export function formatGiB(mib: number): string { return (mib / 1024).toFixed(mib % 1024 === 0 ? 0 : 1); }
export function formatBytesAsGiB(bytes: number): string { return (bytes / 1024 ** 3).toFixed(bytes % (1024 ** 3) === 0 ? 0 : 1); }
