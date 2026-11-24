// Local query/client marker for an HTTP-only session. Never sent as a credential.
export const BROWSER_SESSION = ":mwc-browser-session";

export type OidcAction = "login" | "bind" | "unbind";
export type OidcProofAction = Exclude<OidcAction, "login">;
export type OidcResult = OidcProofAction | "unbound" | "failed";
export type OidcStatus = { enabled: boolean; signed_in: boolean };

const oidcPath = "/api/v1/auth/oidc";

export function oidcResult(search: string): OidcResult | null {
  const result = new URLSearchParams(search).get("oidc");
  return result === "bind" || result === "unbind" || result === "unbound" || result === "failed" ? result : null;
}

export function clearOidcResult(): void {
  const url = new URL(window.location.href);
  url.searchParams.delete("oidc");
  window.history.replaceState(window.history.state, "", url);
}

export async function readOidcStatus(signal?: AbortSignal): Promise<OidcStatus> {
  const response = await fetch(`${oidcPath}/status`, { signal, credentials: "same-origin", cache: "no-store" });
  if (!response.ok) return { enabled: false, signed_in: false };
  const value = await response.json() as Partial<OidcStatus>;
  return { enabled: value.enabled === true, signed_in: value.enabled === true && value.signed_in === true };
}

function oidcPost(operation: "start" | "confirm" | "logout", data: Record<string, string>, headers: Record<string, string> = {}) {
  return fetch(`${oidcPath}/${operation}`, {
    method: "POST",
    credentials: "same-origin",
    redirect: "error",
    cache: "no-store",
    headers: { "Content-Type": "application/json", ...headers },
    body: JSON.stringify(data),
  });
}

export async function startOidc(action: OidcAction): Promise<void> {
  const response = await oidcPost("start", { action });
  if (!response.ok) throw new Error("oidc_start_failed");
  const { authorization_url } = await response.json() as { authorization_url: string };
  const url = new URL(authorization_url);
  if (url.protocol !== "https:" || url.username || url.password) throw new Error("invalid_oidc_authorization_url");
  window.location.assign(url);
}

export function confirmOidc(action: OidcProofAction, apiKey: string): Promise<Response> {
  return oidcPost("confirm", { action }, { Authorization: `Bearer ${apiKey}` });
}

export function logoutOidc(): Promise<Response> {
  return oidcPost("logout", {});
}
