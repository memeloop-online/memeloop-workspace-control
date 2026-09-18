import { StorageMeter as SharedStorageMeter } from "../shared";
import { useI18n } from "../i18n";
import type { Locale } from "../i18n";
import type { WorkspaceStorageTelemetry } from "../types";

export { formatBytes } from "../shared";

interface Props {
  label: string;
  /** Live telemetry; undefined while runtime data is missing (for example stopped). */
  telemetry?: WorkspaceStorageTelemetry;
  /** Configured capacity from the workspace snapshot, shown when telemetry is absent. */
  configuredGiB: number;
  locale: Locale;
  statusOverride?: string;
}

/** Product adapter for the shared storage meter. */
export function StorageMeter({ label, telemetry, configuredGiB, locale, statusOverride }: Props) {
  const { t } = useI18n();
  return (
    <SharedStorageMeter
      label={label}
      telemetry={telemetry}
      configuredGiB={configuredGiB}
      locale={locale}
      statusOverride={statusOverride}
      labels={{
        telemetryStale: t("storageTelemetryStale"),
        telemetryDisabled: t("storageTelemetryDisabled"),
        telemetryAvailable: t("storageTelemetryAvailable"),
        telemetryUnavailable: t("storageTelemetryUnavailable"),
        nodeLocalTelemetryUnavailable: t("nodeLocalStorageTelemetryUnavailable"),
        pressureCritical: t("storagePressureCritical"),
        pressureWarning: t("storagePressureWarning"),
        observedAt: t("observedAt"),
        usageNotObserved: t("usageNotObserved"),
        storageUsage: t("storageUsage"),
        usageOfLimit: t("usageOfLimit"),
      }}
    />
  );
}
