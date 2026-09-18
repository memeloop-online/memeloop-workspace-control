import { Caption1, ProgressBar, Text, Tooltip } from "@fluentui/react-components";

import { formatPercent } from "../workspaceMetrics";
import { useSharedStyles } from "./styles";

export interface ResourceMeterProps {
  label: string;
  /** Pre-formatted actual usage, for example "120m" or "256 MiB". */
  actual: string;
  /** Pre-formatted requested limit shown next to the actual value. */
  requested: string;
  /** Usage percentage 0-100, or null when metrics are unavailable. */
  percent: number | null;
  /** Shown in the tooltip when metrics are unavailable. */
  unavailableLabel: string;
  /** "Usage of limit" caption and progress bar aria label suffix. */
  usageOfLimitLabel: string;
}

/** Consistent actual/requested meter for workspace CPU and memory usage. */
export function ResourceMeter({ label, actual, requested, percent, unavailableLabel, usageOfLimitLabel }: ResourceMeterProps) {
  const styles = useSharedStyles();
  const valueText = percent === null ? `${label}: ${unavailableLabel}` : `${actual} / ${requested}, ${formatPercent(percent)}`;
  return (
    <Tooltip content={valueText} relationship="description">
      <div tabIndex={0} className={styles.meter}>
        <div className={styles.meterHeader}>
          <Text>{label}</Text>
          <Text className={styles.meterValue}>{actual} <Caption1>/ {requested}</Caption1></Text>
        </div>
        <ProgressBar value={percent === null ? undefined : percent / 100} aria-label={`${label} ${usageOfLimitLabel}`} />
        <Caption1 className={styles.meterHint}>{usageOfLimitLabel} · {formatPercent(percent)}</Caption1>
      </div>
    </Tooltip>
  );
}
