import type {
  ApiFailure,
  ApiKeySummary,
  ApiKeyScope,
  AuditPage,
  CreatedApiKey,
  CreateUserInput,
  CreateWorkspace,
  InjectionDraft,
  InjectionScope,
  ImagePolicy,
  MembershipPage,
  Organization,
  OrganizationPage,
  Principal,
  Resources,
  ResolvedInjection,
  Role,
  ScalingStatus,
  StoredInjection,
  UserProfile,
  UserSummary,
  UserPage,
  WebShellTicket,
  WebhookSubscription,
  WorkspaceResponse,
  WorkspacePage,
  WorkspaceRuntime,
  WorkspaceRuntimeEntry,
  WorkspaceTemplate,
} from "./types";
import type { CreatePortMappingInput, PortMapping } from "./portMappings";

const TOKEN_KEY = "mwc.api-token";

/** Options shared by the keyset-paginated list endpoints. */
export interface PageOptions {
  limit?: number;
  cursor?: string;
  search?: string;
}

export interface UsersPageOptions extends PageOptions {
  organization_id?: string;
}

function queryString(values: object): string {
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(values as Record<string, string | number | undefined>)) {
    if (value !== undefined && value !== "") query.set(key, String(value));
  }
  return query.toString();
}

export class ApiClient {
  constructor(readonly token: string) {}

  static savedToken(): string {
    return sessionStorage.getItem(TOKEN_KEY) ?? "";
  }

  static rememberToken(token: string): void {
    sessionStorage.setItem(TOKEN_KEY, token);
  }

  static forgetToken(): void {
    sessionStorage.removeItem(TOKEN_KEY);
  }

  me(): Promise<Principal> {
    return this.request("/api/v1/me");
  }

  profile(): Promise<UserProfile> {
    return this.request("/api/v1/me/profile");
  }

  updateProfile(profile: UserProfile): Promise<UserProfile> {
    return this.request("/api/v1/me/profile", {
      method: "PUT", body: JSON.stringify(profile),
    });
  }

  apiKeys(): Promise<ApiKeySummary[]> {
    return this.request("/api/v1/me/api-keys");
  }

  createApiKey(input: { name: string; scopes: ApiKeyScope[]; expires_at: number }): Promise<CreatedApiKey> {
    return this.request("/api/v1/me/api-keys", {
      method: "POST", body: JSON.stringify(input),
    });
  }

  deleteApiKey(id: string): Promise<void> {
    return this.request(`/api/v1/me/api-keys/${encodeURIComponent(id)}`, { method: "DELETE" });
  }

  /**
   * Return only the requested page's items. Call organizationsPage when the
   * caller needs to continue past this page or distinguish a complete list.
   */
  organizations(options: PageOptions = {}): Promise<Organization[]> {
    return this.organizationsPage(options).then((page) => page.items);
  }

  organizationsPage(options: PageOptions = {}): Promise<OrganizationPage> {
    return this.request(`/api/v1/organizations?${queryString(options)}`);
  }

  createOrganization(name: string): Promise<Organization> {
    return this.request("/api/v1/organizations", {
      method: "POST",
      body: JSON.stringify({ name, owner_user_id: "00000000-0000-0000-0000-000000000000" }),
      idempotent: true,
    });
  }

  updateOrganization(id: string, name: string): Promise<Organization> {
    return this.request(`/api/v1/organizations/${id}`, {
      method: "PUT", body: JSON.stringify({ name }),
    });
  }

  deleteOrganization(id: string): Promise<void> {
    return this.request(`/api/v1/organizations/${id}`, { method: "DELETE" });
  }

  /** Return only one page; use usersPage for cursor metadata. */
  users(options: UsersPageOptions = {}): Promise<UserSummary[]> {
    return this.usersPage(options).then((page) => page.items);
  }

  usersPage(options: UsersPageOptions = {}): Promise<UserPage> {
    return this.request(`/api/v1/admin/users?${queryString(options)}`);
  }

  createUser(input: CreateUserInput): Promise<UserSummary> {
    return this.request("/api/v1/admin/users", {
      method: "POST", body: JSON.stringify(input), idempotent: true,
    });
  }

  updateUser(id: string, input: { display_name?: string; system_admin?: boolean; disabled?: boolean }): Promise<UserSummary> {
    return this.request(`/api/v1/admin/users/${id}`, {
      method: "PUT", body: JSON.stringify(input),
    });
  }

  setMembership(organizationId: string, userId: string, role: Role): Promise<void> {
    return this.request(`/api/v1/organizations/${organizationId}/members/${userId}`, {
      method: "PUT", body: JSON.stringify({ role }), idempotent: true,
    });
  }

  removeMembership(organizationId: string, userId: string): Promise<void> {
    return this.request(`/api/v1/organizations/${organizationId}/members/${userId}`, {
      method: "DELETE", idempotent: true,
    });
  }

  membersPage(organizationId: string, options: PageOptions = {}): Promise<MembershipPage> {
    return this.request(`/api/v1/organizations/${encodeURIComponent(organizationId)}/members?${queryString(options)}`);
  }

  quota(organizationId: string): Promise<Resources | null> {
    return this.request(`/api/v1/organizations/${organizationId}/quota`);
  }

  setQuota(organizationId: string, resources: Resources): Promise<void> {
    return this.request(`/api/v1/organizations/${organizationId}/quota`, {
      method: "PUT", body: JSON.stringify(resources), idempotent: true,
    });
  }

  userQuota(userId: string): Promise<Resources | null> {
    return this.request(`/api/v1/admin/users/${userId}/quota`);
  }

  setUserQuota(userId: string, resources: Resources): Promise<void> {
    return this.request(`/api/v1/admin/users/${userId}/quota`, {
      method: "PUT", body: JSON.stringify(resources), idempotent: true,
    });
  }

  audit(organizationId: string | undefined, options: { limit?: number; offset?: number; action?: string; actor?: string; workspace?: string; q?: string } = {}): Promise<AuditPage> {
    const query = new URLSearchParams();
    if (organizationId) query.set("organization_id", organizationId);
    for (const [key, value] of Object.entries(options)) {
      if (value !== undefined && value !== "") query.set(key, String(value));
    }
    return this.request(`/api/v1/audit?${query}`);
  }

  scaling(): Promise<ScalingStatus> {
    return this.request("/api/v1/admin/scaling");
  }

  images(): Promise<ImagePolicy[]> {
    return this.request("/api/v1/admin/images");
  }

  putImage(image: string, enabled = true): Promise<ImagePolicy> {
    return this.request("/api/v1/admin/images", {
      method: "PUT", body: JSON.stringify({ image, enabled }), idempotent: true,
    });
  }

  templates(organizationId: string): Promise<WorkspaceTemplate[]> {
    return this.request(`/api/v1/templates?organization_id=${organizationId}`);
  }

  createTemplate(input: { organization_id: string | null; yaml: string }): Promise<WorkspaceTemplate> {
    return this.request("/api/v1/templates", {
      method: "POST", body: JSON.stringify(input), idempotent: true,
    });
  }

  replaceTemplate(templateId: string, yaml: string): Promise<WorkspaceTemplate> {
    return this.request(`/api/v1/templates/${templateId}`, {
      method: "PUT", body: JSON.stringify({ yaml }), idempotent: true,
    });
  }

  setTemplateEnabled(templateId: string, enabled: boolean): Promise<WorkspaceTemplate> {
    return this.request(`/api/v1/templates/${templateId}/enabled`, {
      method: "PUT", body: JSON.stringify({ enabled }), idempotent: true,
    });
  }

  deleteTemplate(templateId: string): Promise<void> {
    return this.request(`/api/v1/templates/${templateId}`, { method: "DELETE" });
  }

  webhooks(organizationId: string): Promise<WebhookSubscription[]> {
    return this.request(`/api/v1/webhooks?organization_id=${organizationId}`);
  }

  createWebhook(input: { organization_id: string; url: string; event_prefix: string; signing_secret: string }): Promise<WebhookSubscription> {
    return this.request("/api/v1/webhooks", {
      method: "POST", body: JSON.stringify(input), idempotent: true,
    });
  }

  /**
   * Return only one organization-scoped page. WorkspacePanel owns loading
   * additional pages; callers such as the workspace combobox should use
   * workspacesPage with a search and bounded limit.
   */
  workspaces(organizationId: string, options: PageOptions = {}): Promise<WorkspaceResponse[]> {
    return this.workspacesPage(organizationId, options).then((page) => page.items);
  }

  workspacesPage(organizationId: string, options: PageOptions = {}): Promise<WorkspacePage> {
    return this.request(`/api/v1/workspaces?${queryString({ organization_id: organizationId, ...options })}`);
  }

  createWorkspace(command: CreateWorkspace): Promise<WorkspaceResponse> {
    return this.request("/api/v1/workspaces", {
      method: "POST",
      body: JSON.stringify(command),
      idempotent: true,
    });
  }

  workspaceAction(
    workspaceId: string,
    action: "start" | "stop" | "restart" | "delete",
  ): Promise<WorkspaceResponse> {
    return this.request(
      `/api/v1/workspaces/${workspaceId}/actions/${action}`,
      { method: "POST", idempotent: true },
    );
  }

  workspaceRuntime(workspaceId: string): Promise<WorkspaceRuntime> {
    return this.request(`/api/v1/workspaces/${workspaceId}/runtime`);
  }

  workspaceRuntimes(organizationId: string, workspaceIds: string[]): Promise<WorkspaceRuntimeEntry[]> {
    return this.request(`/api/v1/workspace-runtimes?${queryString({ organization_id: organizationId, workspace_ids: workspaceIds.join(",") })}`);
  }

  issueWebShellTicket(workspaceId: string): Promise<WebShellTicket> {
    return this.request(
      `/api/v1/workspaces/${workspaceId}/web-shell-tickets`,
      { method: "POST" },
    );
  }

  portMappings(workspaceId: string): Promise<PortMapping[]> {
    return this.request(`/api/v1/workspaces/${workspaceId}/port-mappings`);
  }

  createPortMapping(workspaceId: string, input: CreatePortMappingInput): Promise<PortMapping> {
    return this.request(`/api/v1/workspaces/${workspaceId}/port-mappings`, {
      method: "POST", body: JSON.stringify(input), idempotent: true,
    });
  }

  deletePortMapping(workspaceId: string, mappingId: string): Promise<void> {
    return this.request(`/api/v1/workspaces/${workspaceId}/port-mappings/${mappingId}`, {
      method: "DELETE", idempotent: true,
    });
  }

  bootstrapPortMapping(workspaceId: string, mappingId: string): Promise<{ bootstrap_url: string }> {
    return this.request(`/api/v1/workspaces/${workspaceId}/port-mappings/${mappingId}/open`, {
      method: "POST",
    });
  }

  injections(scope: InjectionScope, scopeId: string): Promise<StoredInjection[]> {
    return this.request(`/api/v1/injections/${scope}/${scopeId}`);
  }

  replaceInjection(
    scope: InjectionScope,
    scopeId: string,
    item: InjectionDraft,
  ): Promise<StoredInjection> {
    return this.request(
      `/api/v1/injections/${scope}/${scopeId}/${encodeURIComponent(item.key)}`,
      { method: "PUT", body: JSON.stringify(item), idempotent: true },
    );
  }

  deleteInjection(scope: InjectionScope, scopeId: string, key: string): Promise<void> {
    return this.request(
      `/api/v1/injections/${scope}/${scopeId}/${encodeURIComponent(key)}`,
      { method: "DELETE", idempotent: true },
    );
  }

  previewInjections(input: {
    organization_id: string | null;
    user_id: string;
    workspace_id: string | null;
    organization_injection_refs?: string[] | null;
    user_injection_refs?: string[] | null;
    inline_workspace_injections: InjectionDraft[];
  }): Promise<ResolvedInjection[]> {
    return this.request("/api/v1/injections/preview", {
      method: "POST",
      body: JSON.stringify(input),
    });
  }

  private async request<T>(
    path: string,
    init: RequestInit & { idempotent?: boolean } = {},
  ): Promise<T> {
    const headers = new Headers(init.headers);
    headers.set("Authorization", `Bearer ${this.token}`);
    if (init.body) headers.set("Content-Type", "application/json");
    if (init.idempotent) headers.set("Idempotency-Key", crypto.randomUUID());
    const response = await fetch(path, { ...init, headers });
    if (!response.ok) {
      let failure: ApiFailure = {};
      try {
        failure = (await response.json()) as ApiFailure;
      } catch {
        // Keep the stable HTTP fallback below.
      }
      const error = new Error(
        failure.error?.message ?? `请求失败（HTTP ${response.status}）`,
      ) as Error & { status: number; code?: string };
      error.status = response.status;
      error.code = failure.error?.code;
      throw error;
    }
    if (response.status === 204) return undefined as T;
    return (await response.json()) as T;
  }
}
