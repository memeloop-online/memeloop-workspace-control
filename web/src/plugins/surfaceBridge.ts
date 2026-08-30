import type { PluginApiBridgeRequest, PluginApiRequestMethod, PluginBridgeRequest } from "./types";

export const SUPPORTED_PLUGIN_BRIDGE_METHODS = ["theme.read", "plugin_api.request"] as const;
export const MAX_PLUGIN_API_REQUEST_BYTES = 256 * 1024;
export const MAX_PLUGIN_API_RESPONSE_BYTES = 1024 * 1024;
const PLUGIN_API_METHODS = new Set<PluginApiRequestMethod>(["GET", "POST", "PUT", "PATCH", "DELETE"]);

export function parsePluginBridgeRequest(
  value: unknown,
  nonce: string,
  allowedMethods: string[],
): PluginBridgeRequest | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const message = value as Record<string, unknown>;
  if (message.type !== "mwc:bridge" || message.nonce !== nonce) return null;
  if (typeof message.request_id !== "string" || !/^[A-Za-z0-9._-]{1,96}$/u.test(message.request_id)) return null;
  if (typeof message.method !== "string"
    || !SUPPORTED_PLUGIN_BRIDGE_METHODS.some((method) => method === message.method)
    || !allowedMethods.includes(message.method)) return null;
  return { request_id: message.request_id, method: message.method, payload: message.payload };
}

export function parsePluginApiBridgeRequest(
  value: unknown,
  declaredRouteIds: string[],
): PluginApiBridgeRequest | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const request = value as Record<string, unknown>;
  if (Object.keys(request).some((key) => !["route_id", "method", "path", "body"].includes(key))) return null;
  if (typeof request.route_id !== "string" || !declaredRouteIds.includes(request.route_id)) return null;
  if (typeof request.method !== "string" || !PLUGIN_API_METHODS.has(request.method as PluginApiRequestMethod)) return null;
  if (typeof request.path !== "string") return null;
  const path = safePluginApiRelativePath(request.path);
  if (path === null || ((request.method === "GET" || request.method === "DELETE") && request.body !== undefined)) return null;
  if (request.body !== undefined) {
    try {
      if (new TextEncoder().encode(JSON.stringify(request.body)).byteLength > MAX_PLUGIN_API_REQUEST_BYTES) return null;
    } catch {
      return null;
    }
  }
  return {
    route_id: request.route_id,
    method: request.method as PluginApiRequestMethod,
    path,
    ...(request.body === undefined ? {} : { body: request.body }),
  };
}

export function safePluginApiRelativePath(value: string): string | null {
  const relative = value.startsWith("/") ? value.slice(1) : value;
  if (relative.length > 1024 || relative.includes("?") || relative.includes("#") || relative.includes("\\") || /[\u0000-\u001f\u007f]/u.test(relative)) return null;
  if (!relative) return "";
  const segments = relative.split("/");
  if (segments.some((segment) => !segment || segment === "." || segment === "..")) return null;
  return segments.map((segment) => encodeURIComponent(segment)).join("/");
}

export function currentPluginTheme(root: Pick<HTMLElement, "dataset">): "light" | "dark" {
  return root.dataset.theme === "light" ? "light" : "dark";
}

export function safePluginSessionPath(value: string, origin: string): string | null {
  return safeSameOriginPath(value, origin, "/api/v1/plugin-ui/");
}

function safeSameOriginPath(value: string, origin: string, prefix: string): string | null {
  try {
    const url = new URL(value, origin);
    if (url.origin !== origin || !url.pathname.startsWith(prefix)) return null;
    return `${url.pathname}${url.search}`;
  } catch {
    return null;
  }
}
