import { Caption1, Card, Text, Title3 } from "@fluentui/react-components";
import type { ReactNode } from "react";

import type { WorkspaceState } from "../types";
import { WorkspaceStateBadge } from "./WorkspaceStateBadge";
import { useSharedStyles } from "./styles";

export interface WorkspaceStatusCardProps {
  title: string;
  shortId: string;
  state: WorkspaceState;
  /** Visible label per state; falls back to the raw state string when missing. */
  stateLabels: Partial<Record<WorkspaceState, string>>;
  /** Pre-formatted metadata strings, for example user, access mode, or image. */
  metadata: readonly string[];
  /** Meter nodes (ResourceMeter, StorageMeter, …) rendered in the resource grid. */
  meters: readonly ReactNode[];
}

/** Read-only workspace summary card: identity, state, metadata, and usage meters. */
export function WorkspaceStatusCard({ title, shortId, state, stateLabels, metadata, meters }: WorkspaceStatusCardProps) {
  const styles = useSharedStyles();
  return (
    <Card className={styles.card} appearance="filled-alternative">
      <div className={styles.cardHeader}>
        <div className={styles.cardTitle}>
          <Title3 as="h2" className={styles.titleText}>{title}</Title3>
          <Caption1 className={styles.idText}>{shortId}</Caption1>
        </div>
        <WorkspaceStateBadge state={state} labels={stateLabels} />
      </div>
      {metadata.length > 0 && (
        <div className={styles.metadata}>
          {metadata.map((entry, index) => <Text key={index}>{entry}</Text>)}
        </div>
      )}
      {meters.length > 0 && (
        <div className={styles.resourceGrid}>
          {meters.map((meter, index) => <span key={index} style={{ display: "contents" }}>{meter}</span>)}
        </div>
      )}
    </Card>
  );
}
