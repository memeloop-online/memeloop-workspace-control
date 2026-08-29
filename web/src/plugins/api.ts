import type {
  PluginConfiguration,
  PluginManifest,
  PutPluginConfiguration,
} from "./types";

type ApiFailure = { error?: { code?: string; message?: string } };

export class PluginApi {
  constructor(private readonly token: string) {}

  manifests(): Promise<PluginManifest[]> {
    return this.request("/api/v1/plugins");
  }

  configuration(pluginId: string, organizationId: string | null): Promise<PluginConfiguration> {
    return this.request(this.configurationPath(pluginId, organizationId));
  }

  putConfiguration(
    pluginId: string,
    organizationId: string | null,
    body: PutPluginConfiguration,
  ): Promise<PluginConfiguration> {
    return this.request(this.configurationPath(pluginId, organizationId), {
      method: "PUT",
      body: JSON.stringify(body),
      idempotent: true,
    });
  }

  deleteConfiguration(
    pluginId: string,
    organizationId: string | null,
    expectedVersion: number,
  ): Promise<PluginConfiguration> {
    return this.request(this.configurationPath(pluginId, organizationId), {
      method: "DELETE",
      body: JSON.stringify({ expected_version: expectedVersion }),
      idempotent: true,
    });
  }

  private configurationPath(pluginId: string, organizationId: string | null): string {
    const base = `/api/v1/plugins/${encodeURIComponent(pluginId)}/configuration`;
    return organizationId
      ? `${base}?organization_id=${encodeURIComponent(organizationId)}`
      : base;
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
        // Preserve the stable HTTP fallback for non-JSON reverse-proxy errors.
      }
      const error = new Error(failure.error?.message ?? `HTTP ${response.status}`);
      Object.assign(error, { code: failure.error?.code, status: response.status });
      throw error;
    }
    return (await response.json()) as T;
  }
}
