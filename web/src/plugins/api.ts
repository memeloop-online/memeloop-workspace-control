import type {
  PluginConfiguration,
  PluginApiBridgeRequest,
  PluginApiBridgeResponse,
  PluginInspection,
  PluginManifest,
  PluginSurfaceSession,
  PutPluginConfiguration,
} from "./types";

const MAX_PLUGIN_API_REQUEST_BYTES = 256 * 1024;
const MAX_PLUGIN_API_RESPONSE_BYTES = 1024 * 1024;

type ApiFailure = { error?: { code?: string; message?: string } };

export class PluginApi {
  private readonly token: string;

  constructor(token: string) {
    this.token = token;
  }

  manifests(): Promise<PluginManifest[]> {
    return this.request("/api/v1/plugins");
  }

  inspectLocalPackage(manifest: File, component: File | null, assets: File[]): Promise<PluginInspection> {
    const body = new FormData();
    body.set("manifest", manifest);
    if (component) body.set("component", component);
    assets.forEach((asset) => body.append("asset", asset));
    return this.request("/api/v1/plugins/inspections/upload", { method: "POST", body, idempotent: true });
  }

  inspectUrl(url: string, expectedSha256: string): Promise<PluginInspection> {
    return this.request("/api/v1/plugins/inspections/url", {
      method: "POST",
      body: JSON.stringify({ url, expected_sha256: expectedSha256 }),
      idempotent: true,
    });
  }

  inspectGithubRelease(repository: string, tag: string, asset: string, expectedSha256: string): Promise<PluginInspection> {
    return this.request("/api/v1/plugins/inspections/github-release", {
      method: "POST",
      body: JSON.stringify({ repository, tag, asset, expected_sha256: expectedSha256 }),
      idempotent: true,
    });
  }

  install(inspection: PluginInspection, approvedContributions: string[], enabled: boolean): Promise<PluginManifest> {
    return this.request("/api/v1/plugins/installs", {
      method: "POST",
      body: JSON.stringify({
        inspection_id: inspection.inspection_id,
        expected_digest: inspection.digest,
        expected_package_version: inspection.current_package_version,
        approved_contributions: approvedContributions,
        enabled,
      }),
      idempotent: true,
    });
  }

  setEnabled(pluginId: string, enabled: boolean, expectedVersion: number): Promise<PluginManifest> {
    return this.request(`/api/v1/plugins/${encodeURIComponent(pluginId)}/enabled`, {
      method: "PUT",
      body: JSON.stringify({ enabled, expected_version: expectedVersion }),
      idempotent: true,
    });
  }

  uninstall(pluginId: string, expectedVersion: number): Promise<void> {
    return this.request(`/api/v1/plugins/${encodeURIComponent(pluginId)}`, {
      method: "DELETE",
      body: JSON.stringify({ expected_version: expectedVersion }),
      idempotent: true,
    });
  }

  createSurfaceSession(pluginId: string, surfaceId: string): Promise<PluginSurfaceSession> {
    return this.request(`/api/v1/plugins/${encodeURIComponent(pluginId)}/ui-surfaces/${encodeURIComponent(surfaceId)}/sessions`, {
      method: "POST", idempotent: true,
    });
  }

  async invokePluginRoute(
    pluginId: string,
    request: PluginApiBridgeRequest,
    organizationId: string | null,
  ): Promise<PluginApiBridgeResponse> {
    const route = `/api/v1/plugin-api/${encodeURIComponent(pluginId)}/${encodeURIComponent(request.route_id)}/${request.path}`;
    const path = organizationId ? `${route}?organization_id=${encodeURIComponent(organizationId)}` : route;
    const headers = new Headers({ Authorization: `Bearer ${this.token}` });
    const body = request.body === undefined ? undefined : JSON.stringify(request.body);
    if (body !== undefined && new TextEncoder().encode(body).byteLength > MAX_PLUGIN_API_REQUEST_BYTES) throw new Error("plugin_api_request_too_large");
    if (body !== undefined) headers.set("Content-Type", "application/json");
    const response = await fetch(path, { method: request.method, headers, body, credentials: "same-origin" });
    const contentType = response.headers.get("Content-Type") ?? "application/octet-stream";
    if (!(contentType.startsWith("application/json") || contentType.startsWith("text/"))) throw new Error("plugin_api_response_type_invalid");
    return {
      status: response.status,
      content_type: contentType,
      body: await readLimitedText(response, MAX_PLUGIN_API_RESPONSE_BYTES),
    };
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
    if (init.body && !(init.body instanceof FormData)) headers.set("Content-Type", "application/json");
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
    if (response.status === 204) return undefined as T;
    return (await response.json()) as T;
  }
}

async function readLimitedText(response: Response, limit: number): Promise<string> {
  const declared = Number(response.headers.get("Content-Length"));
  if (Number.isFinite(declared) && declared > limit) throw new Error("plugin_api_response_too_large");
  if (!response.body) return "";
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let size = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > limit) {
      await reader.cancel();
      throw new Error("plugin_api_response_too_large");
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
  return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
}
