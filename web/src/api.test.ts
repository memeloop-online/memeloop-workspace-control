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

test("organization usage summary uses the dedicated aggregate endpoint", async () => {
  const originalFetch = globalThis.fetch;
  let request = "";
  globalThis.fetch = async (input) => {
    request = String(input);
    return Response.json({ total_count: 0, state_counts: {}, requested: {}, actual: {}, observed_at: null, availability: {}, coverage: {} });
  };
  try {
    await new ApiClient("operator-token").usageSummary("organization / one");
    assert.equal(request, "/api/v1/organizations/organization%20%2F%20one/usage-summary");
  } finally { globalThis.fetch = originalFetch; }
});

test("scaling updates send the job worker concurrency and return the refreshed status", async () => {
  const originalFetch = globalThis.fetch;
  const requests: Array<{ input: RequestInfo | URL; init?: RequestInit }> = [];
  globalThis.fetch = async (input, init) => {
    requests.push({ input, init });
    return Response.json({
      database_mode: "sqlite",
      configured_replicas: 1,
      schema_version: 12,
      job_worker_concurrency: 64,
      jobs: { pending: 0, running: 0, completed: 0, failed: 0 },
    });
  };
  try {
    const status = await new ApiClient("operator-token").updateScaling(64);
    assert.equal(status.job_worker_concurrency, 64);
    assert.equal(String(requests[0]?.input), "/api/v1/admin/scaling");
    assert.equal(requests[0]?.init?.method, "PUT");
    assert.deepEqual(JSON.parse(String(requests[0]?.init?.body)), { job_worker_concurrency: 64 });
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("workspace runtime payload preserves the scheduled Kubernetes node name", async () => {
  const originalFetch = globalThis.fetch;
  let request = "";
  globalThis.fetch = async (input) => {
    request = String(input);
    return Response.json([{ workspace_id: "workspace-1", runtime: { node_name: "worker-a" } }]);
  };
  try {
    const entries = await new ApiClient("operator-token").workspaceRuntimes("organization-1", ["workspace-1"]);
    assert.equal(entries[0]?.runtime.node_name, "worker-a");
    assert.equal(request, "/api/v1/workspace-runtimes?organization_id=organization-1&workspace_ids=workspace-1");
  } finally { globalThis.fetch = originalFetch; }
});


test("scaling partial updates preserve omitted settings in the request", async () => {
  const originalFetch = globalThis.fetch;
  const bodies: unknown[] = [];
  globalThis.fetch = async (_input, init) => {
    bodies.push(JSON.parse(String(init?.body)));
    return Response.json({ cpu_overcommit_ratio: 20, job_worker_concurrency: 64 });
  };
  try {
    const api = new ApiClient("operator-token");
    assert.equal((await api.updateScaling({ cpu_overcommit_ratio: 20 })).cpu_overcommit_ratio, 20);
    await api.updateScaling({ job_worker_concurrency: 64 });
    await api.updateScaling({ cpu_overcommit_ratio: 20, job_worker_concurrency: 64 });
    assert.deepEqual(bodies, [{ cpu_overcommit_ratio: 20 }, { job_worker_concurrency: 64 }, { cpu_overcommit_ratio: 20, job_worker_concurrency: 64 }]);
  } finally { globalThis.fetch = originalFetch; }
});

test("workspace page requests serialize the sort direction", async () => {
  const originalFetch = globalThis.fetch;
  const requests: string[] = [];
  globalThis.fetch = async (input) => {
    requests.push(String(input));
    return Response.json({ items: [], next_cursor: null, summary: { total_count: 0, requested: {}, temporary_requested_gib: 0, state_counts: {} } });
  };
  try {
    const api = new ApiClient("operator-token");
    await api.workspacesPage("org-1", { limit: 50, cursor: "cursor-1", search: "ops", sort: "oldest" });
    await api.workspacesPage("org-1", { sort: "newest" });
    await api.workspaces("org-1");
    assert.equal(requests[0], "/api/v1/workspaces?organization_id=org-1&limit=50&cursor=cursor-1&search=ops&sort=oldest");
    assert.equal(requests[1], "/api/v1/workspaces?organization_id=org-1&sort=newest");
    assert.equal(requests[2], "/api/v1/workspaces?organization_id=org-1", "omitted sort relies on the backend newest-first default");
  } finally { globalThis.fetch = originalFetch; }
});
