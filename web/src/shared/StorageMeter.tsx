import { Caption1, ProgressBar, Text, Tooltip } from "@fluentui/react-components";

import type { WorkspaceStorageTelemetry } from "../types";
import { formatPercent } from "../workspaceMetrics";
import { useSharedStyles } from "./styles";

export interface StorageMeterLabels {
  telemetryStale: string;
  telemetryDisabled: string;
  telemetryAvailable: string;
  telemetryUnavailable: string;
  nodeLocalTelemetryUnavailable: string;
  pressureCritical: string;
  pressureWarning: string;
  observedAt: string;
  usageNotObserved: string;
  storageUsage: string;
  usageOfLimit: string;
}

export interface StorageMeterProps {
  label: string;
  /** Live telemetry; undefined while runtime data is missing (for example stopped). */
  telemetry?: WorkspaceStorageTelemetry;
  /** Configured capacity from the workspace snapshot, shown when telemetry is absent. */
  configuredGiB: number;
  /** Locale used to format the observation timestamp. */
  locale: string;
  labels: StorageMeterLabels;
  /** Product-owned lifecycle text, such as a released stopped-runtime volume. */
  statusOverride?: string;
}

/** Consistent capacity/usage meter for workspace storage, persistent or temporary. */
export function StorageMeter({ label, telemetry, configuredGiB, locale, labels, statusOverride }: StorageMeterProps) {
  const styles = useSharedStyles();
  const coverage = telemetry?.coverage ?? "unavailable";
  const usable = coverage === "exact" || coverage === "stale";
  const used = usable ? telemetry?.used_bytes ?? null : null;
  const capacity = usable ? telemetry?.capacity_bytes ?? null : null;
  const percent = used !== null && capacity !== null && capacity > 0
    ? Math.min(100, Math.max(0, (used / capacity) * 100))
    : null;
  const capacityText = capacity !== null
    ? formatBytes(capacity)
    : telemetry
      ? formatBytes(telemetry.configured_bytes)
      : `${configuredGiB} GiB`;
  const telemetryStatus = coverage === "stale"
    ? labels.telemetryStale
    : coverage === "disabled"
      ? labels.telemetryDisabled
      : coverage === "exact"
        ? labels.telemetryAvailable
        : telemetry?.backing === "node_local"
          ? labels.nodeLocalTelemetryUnavailable
          : labels.telemetryUnavailable;
  const statusText = statusOverride ?? telemetryStatus;
  const pressureText = telemetry?.pressure === "critical"
    ? labels.pressureCritical
    : telemetry?.pressure === "warning"
      ? labels.pressureWarning
      : null;
  const observed = telemetry?.observed_at
    ? `${labels.observedAt} ${new Date(telemetry.observed_at * 1_000).toLocaleString(locale)}`
    : labels.usageNotObserved;
  const detail = `${label}: ${used === null ? "—" : formatBytes(used)} / ${capacityText}. ${statusText}.${pressureText ? ` ${pressureText}.` : ""} ${observed}.`;
  return (
    <Tooltip content={detail} relationship="description">
      <div tabIndex={0} className={styles.meter}>
        <div className={styles.meterHeader}>
          <Text>{label}</Text>
          <Text className={styles.meterValue}>{used === null ? "—" : formatBytes(used)} <Caption1>/ {capacityText}</Caption1></Text>
        </div>
        <ProgressBar
          value={percent === null ? undefined : percent / 100}
          aria-label={`${label} ${labels.storageUsage}`}
          {...(telemetry?.pressure === "critical" ? { color: "error" as const } : telemetry?.pressure === "warning" ? { color: "warning" as const } : {})}
        />
        <Caption1 className={styles.meterHint}>{percent === null ? statusText : `${labels.usageOfLimit} · ${formatPercent(percent)}`}</Caption1>
        {pressureText && <Caption1 className={styles.meterHint}>{pressureText}</Caption1>}
        {percent !== null && coverage !== "exact" && <Caption1 className={styles.meterHint}>{statusText}</Caption1>}
      </div>
    </Tooltip>
  );
}

export function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value < 0) return "—";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let scaled = value;
  let unit = 0;
  while (scaled >= 1024 && unit < units.length - 1) { scaled /= 1024; unit += 1; }
  return `${scaled >= 10 || unit === 0 ? scaled.toFixed(0) : scaled.toFixed(1)} ${units[unit]}`;
}
