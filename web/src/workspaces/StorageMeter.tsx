import { Caption1, ProgressBar, Text, Tooltip } from "@fluentui/react-components";

import { useI18n } from "../i18n";
import type { Locale } from "../i18n";
import type { WorkspaceStorageTelemetry } from "../types";
import { formatPercent } from "../workspaceMetrics";
import { useWorkspaceStyles } from "./workspaceStyles";

interface Props {
  label: string;
  /** Live telemetry; undefined while runtime data is missing (for example stopped). */
  telemetry?: WorkspaceStorageTelemetry;
  /** Configured capacity from the workspace snapshot, shown when telemetry is absent. */
  configuredGiB: number;
  locale: Locale;
}

/** Consistent capacity/usage meter for workspace storage, persistent or temporary. */
export function StorageMeter({ label, telemetry, configuredGiB, locale }: Props) {
  const { t } = useI18n();
  const styles = useWorkspaceStyles();
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
  const statusText = coverage === "stale"
    ? t("storageTelemetryStale")
    : coverage === "disabled"
      ? t("storageTelemetryDisabled")
      : coverage === "exact"
        ? t("storageTelemetryAvailable")
        : t("storageTelemetryUnavailable");
  const pressureText = telemetry?.pressure === "critical"
    ? t("storagePressureCritical")
    : telemetry?.pressure === "warning"
      ? t("storagePressureWarning")
      : null;
  const observed = telemetry?.observed_at
    ? `${t("observedAt")} ${new Date(telemetry.observed_at * 1_000).toLocaleString(locale)}`
    : t("usageNotObserved");
  const detail = `${label}: ${used === null ? "—" : formatBytes(used)} / ${capacityText}. ${statusText}.${pressureText ? ` ${pressureText}.` : ""} ${observed}.`;
  return (
    <Tooltip content={detail} relationship="description">
      <div className={styles.meter}>
        <div className={styles.meterHeader}>
          <Text>{label}</Text>
          <Text className={styles.meterValue}>{used === null ? "—" : formatBytes(used)} <Caption1>/ {capacityText}</Caption1></Text>
        </div>
        <ProgressBar
          value={percent === null ? undefined : percent / 100}
          aria-label={`${label} ${t("storageUsage")}`}
          {...(telemetry?.pressure === "critical" ? { color: "error" as const } : telemetry?.pressure === "warning" ? { color: "warning" as const } : {})}
        />
        <Caption1 className={styles.meterHint}>{percent === null ? statusText : `${t("usageOfLimit")} · ${formatPercent(percent)}`}</Caption1>
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
