import { Badge } from "@fluentui/react-components";

import type { WorkspaceState } from "../types";

export interface WorkspaceStateBadgeProps {
  state: WorkspaceState;
  /** Visible label per state; falls back to the raw state string when missing. */
  labels: Partial<Record<WorkspaceState, string>>;
}

/** Tinted state badge with the same color semantics as the workspace card. */
export function WorkspaceStateBadge({ state, labels }: WorkspaceStateBadgeProps) {
  const color = state === "ready"
    ? "success"
    : state === "failed" || state === "deleting"
      ? "danger"
      : state === "stopped" || state === "deleted"
        ? "informative"
        : "warning";
  return <Badge appearance="tint" color={color}>{labels[state] ?? state}</Badge>;
}
