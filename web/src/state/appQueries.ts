import { useQuery } from "@tanstack/react-query";
import type { ApiClient } from "../api";
import type { OrganizationPage, Principal, WorkspacePage } from "../types";

const PREVIEW_LIMIT = 30;
const ORGANIZATION_LIMIT = 50;

/**
 * Authentication changes clear the QueryClient before the next request. Keep
 * bearer tokens out of query keys so credentials never appear in developer
 * tooling or diagnostic snapshots.
 */
export const principalQueryKey = () => ["app", "principal"] as const;
export const organizationsQueryKey = () => ["app", "organizations", ORGANIZATION_LIMIT] as const;
export const workspacePreviewQueryKey = (organizationId: string) => [
  "app",
  "workspace-preview",
  organizationId,
  PREVIEW_LIMIT,
] as const;

export function usePrincipalQuery(api: ApiClient, enabled: boolean) {
  return useQuery<Principal>({
    queryKey: principalQueryKey(),
    queryFn: () => api.me(),
    enabled: enabled && Boolean(api.token),
  });
}

export function useOrganizationsQuery(api: ApiClient, enabled: boolean) {
  return useQuery<OrganizationPage>({
    queryKey: organizationsQueryKey(),
    queryFn: () => api.organizationsPage({ limit: ORGANIZATION_LIMIT }),
    enabled: enabled && Boolean(api.token),
  });
}

export function useWorkspacePreviewQuery(api: ApiClient, organizationId: string, enabled: boolean) {
  return useQuery<WorkspacePage>({
    queryKey: workspacePreviewQueryKey(organizationId),
    queryFn: () => api.workspacesPage(organizationId, { limit: PREVIEW_LIMIT }),
    enabled: enabled && Boolean(api.token) && Boolean(organizationId),
    refetchInterval: 10_000,
    refetchIntervalInBackground: false,
  });
}
