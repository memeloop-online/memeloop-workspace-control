import type { WorkspaceState, WorkspaceSummary } from "../types";

export function workspaceStateCounts(summary: WorkspaceSummary | null): Partial<Record<WorkspaceState, number>> {
  return summary?.state_counts ?? {};
}
