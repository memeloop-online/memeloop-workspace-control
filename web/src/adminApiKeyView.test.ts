import assert from "node:assert/strict";
import test from "node:test";
import { applyLocalRevocations, getApiKeyStatus, prependCreatedApiKey } from "./apiKeyStatus.ts";
import type { AdminApiKey, ApiKeySummary, CreatedApiKey } from "./types.ts";

const baseKey: ApiKeySummary = {
  id: "key-1",
  name: "Daily automation",
  prefix: "mwc_example…",
  last_used_at: null,
  created_at: 10,
  scopes: ["read_workspace"],
  expires_at: 200,
  allowed_template_ids: null,
  revoked_at: null,
};

const createdKey: CreatedApiKey = { ...baseKey, id: "key-created", name: "Fresh key", token: "mwc_plaintext" };

test("API-key status prefers revocation over expiry", () => {
  assert.equal(getApiKeyStatus(baseKey, 100), "active");
  assert.equal(getApiKeyStatus(baseKey, 200), "expired");
  assert.equal(getApiKeyStatus({ ...baseKey, revoked_at: 150 }, 200), "revoked");
});

test("a successful local revocation remains visible when refresh fails", () => {
  const [updated] = applyLocalRevocations([baseKey], new Map([[baseKey.id, 150]]));
  assert.equal(updated?.revoked_at, 150);
  assert.equal(baseKey.revoked_at, null);
});

test("a newly-created key remains copyable when the refresh is briefly stale", () => {
  const stale: AdminApiKey[] = [{ ...baseKey, token: null }];
  const [first] = prependCreatedApiKey(stale, createdKey);
  assert.equal(first?.id, createdKey.id);
  assert.equal(first?.token, createdKey.token);
});
