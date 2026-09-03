import type { ApiKeySummary } from "./types";

export type ApiKeyStatus = "revoked" | "expired" | "active";

export function getApiKeyStatus(
  key: Pick<ApiKeySummary, "revoked_at" | "expires_at">,
  now = Math.floor(Date.now() / 1_000),
): ApiKeyStatus {
  if (key.revoked_at !== null) return "revoked";
  if (key.expires_at !== null && key.expires_at <= now) return "expired";
  return "active";
}

export function applyLocalRevocations(
  items: ApiKeySummary[],
  localRevocations: ReadonlyMap<string, number>,
): ApiKeySummary[] {
  return items.map((key) => {
    const revokedAt = localRevocations.get(key.id);
    return revokedAt === undefined || key.revoked_at !== null
      ? key
      : { ...key, revoked_at: revokedAt };
  });
}
