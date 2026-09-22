import type { AdminApiKey, ApiKeySummary, CreatedApiKey } from "./types";

/** Keep a newly-created plaintext token visible if the follow-up page read is briefly stale. */
export function prependCreatedApiKey(items: AdminApiKey[], created: CreatedApiKey): AdminApiKey[] {
  return [created, ...items.filter((item) => item.id !== created.id)];
}

export type ApiKeyStatus = "revoked" | "expired" | "active";

export function getApiKeyStatus(
  key: Pick<ApiKeySummary, "revoked_at" | "expires_at">,
  now = Math.floor(Date.now() / 1_000),
): ApiKeyStatus {
  if (key.revoked_at !== null) return "revoked";
  if (key.expires_at !== null && key.expires_at <= now) return "expired";
  return "active";
}

export function applyLocalRevocations<T extends ApiKeySummary>(
  items: T[],
  localRevocations: ReadonlyMap<string, number>,
): T[] {
  return items.map((key) => {
    const revokedAt = localRevocations.get(key.id);
    return revokedAt === undefined || key.revoked_at !== null
      ? key
      : { ...key, revoked_at: revokedAt };
  });
}
