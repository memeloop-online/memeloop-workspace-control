import assert from "node:assert/strict";
import test from "node:test";
import { ApiClient } from "./api.ts";

test("administrator API-key requests page summaries and send the revocation reason", async () => {
  const originalFetch = globalThis.fetch;
  const requests: Array<{ input: RequestInfo | URL; init?: RequestInit }> = [];
  globalThis.fetch = async (input, init) => {
    requests.push({ input, init });
    if (requests.length === 1) return Response.json({
      items: [{
        id: "key-1",
        name: "CI runner",
        prefix: "mwc_abc",
        scopes: ["read_workspace"],
        created_at: 100,
        last_used_at: null,
        expires_at: 200,
        revoked_at: null,
      }],
      next_cursor: "next-page",
    });
    return new Response(null, { status: 204 });
  };

  try {
    const api = new ApiClient("operator-token");
    const page = await api.adminUserApiKeys("user-1", { status: "all", limit: 25, cursor: "prior-page" });
    await api.revokeAdminUserApiKey("user-1", "key-1", "No longer needed");

    assert.equal(page.next_cursor, "next-page");
    assert.equal(page.items[0]?.prefix, "mwc_abc");
    assert.equal(Object.hasOwn(page.items[0] ?? {}, "token"), false);
    assert.equal(String(requests[0]?.input), "/api/v1/admin/users/user-1/api-keys?status=all&limit=25&cursor=prior-page");
    assert.equal(requests[1]?.init?.method, "DELETE");
    assert.deepEqual(JSON.parse(String(requests[1]?.init?.body)), { reason: "No longer needed" });
  } finally {
    globalThis.fetch = originalFetch;
  }
});
