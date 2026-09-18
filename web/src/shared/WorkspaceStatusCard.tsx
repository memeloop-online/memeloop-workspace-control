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
  metadata: readonly ReactNode[];
  /** Meter nodes (ResourceMeter, StorageMeter, …) rendered in the resource grid. */
  meters: readonly ReactNode[];
  /** Product controls and expandable details rendered after the summary. */
  children?: ReactNode;
  titleId?: string;
  resources?: ReactNode;
}

/** Read-only workspace summary card: identity, state, metadata, and usage meters. */
export function WorkspaceStatusCard({ title, shortId, state, stateLabels, metadata, meters, children, titleId, resources }: WorkspaceStatusCardProps) {
  const styles = useSharedStyles();
  return (
    <Card className={styles.card} appearance="filled-alternative">
      <div className={styles.cardHeader}>
        <div className={styles.cardTitle}>
          <Title3 as="h2" id={titleId} className={styles.titleText}>{title}</Title3>
          <Caption1 className={styles.idText}>{shortId}</Caption1>
        </div>
        <WorkspaceStateBadge state={state} labels={stateLabels} />
      </div>
      {metadata.length > 0 && (
        <div className={styles.metadata}>
          {metadata.map((entry, index) => typeof entry === "string" ? <Text key={index}>{entry}</Text> : <span key={index} style={{ display: "contents" }}>{entry}</span>)}
        </div>
      )}
      {meters.length > 0 && (
        <div className={styles.resourceGrid}>
          {meters.map((meter, index) => <span key={index} style={{ display: "contents" }}>{meter}</span>)}
        </div>
      )}
      {resources}
      {children}
    </Card>
  );
}
