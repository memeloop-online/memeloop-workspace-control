import type {
  ApiFailure,
  AuditRecord,
  CreateWorkspace,
  InjectionDraft,
  InjectionScope,
  ImagePolicy,
  Organization,
  Principal,
  Resources,
  ResolvedInjection,
  Role,
  ScalingStatus,
  StoredInjection,
  UserSummary,
  WebShellTicket,
  WebhookSubscription,
  WorkspaceResponse,
  WorkspaceRuntime,
  WorkspaceRuntimeEntry,
  WorkspaceTemplate,
} from "./types";

const TOKEN_KEY = "mwc.api-token";

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

  organizations(): Promise<Organization[]> {
    return this.request("/api/v1/organizations");
  }

  createOrganization(name: string): Promise<Organization> {
    return this.request("/api/v1/organizations", {
      method: "POST",
      body: JSON.stringify({ name, owner_user_id: "00000000-0000-0000-0000-000000000000" }),
      idempotent: true,
    });
  }

  users(): Promise<UserSummary[]> {
    return this.request("/api/v1/admin/users");
  }

  createUser(displayName: string, token: string): Promise<UserSummary> {
    return this.request("/api/v1/admin/users", {
      method: "POST", body: JSON.stringify({ display_name: displayName, token, system_admin: false }), idempotent: true,
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

  audit(organizationId: string): Promise<AuditRecord[]> {
    return this.request(`/api/v1/audit?organization_id=${organizationId}`);
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

  workspaces(organizationId: string): Promise<WorkspaceResponse[]> {
    return this.request(
      `/api/v1/workspaces?organization_id=${encodeURIComponent(organizationId)}`,
    );
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

  workspaceRuntimes(organizationId: string): Promise<WorkspaceRuntimeEntry[]> {
    return this.request(`/api/v1/workspace-runtimes?organization_id=${encodeURIComponent(organizationId)}`);
  }

  issueWebShellTicket(workspaceId: string): Promise<WebShellTicket> {
    return this.request(
      `/api/v1/workspaces/${workspaceId}/web-shell-tickets`,
      { method: "POST" },
    );
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
      throw new Error(
        failure.error?.message ?? `请求失败（HTTP ${response.status}）`,
      );
    }
    if (response.status === 204) return undefined as T;
    return (await response.json()) as T;
  }
}
