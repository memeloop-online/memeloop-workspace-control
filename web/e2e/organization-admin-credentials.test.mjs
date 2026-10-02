import assert from "node:assert/strict";
import { test } from "node:test";
import { chromium } from "playwright-core";

const baseUrl = process.env.E2E_BASE_URL ?? "http://127.0.0.1:4173";
const organizationId = "organization-fixture";
const ownUserId = "organization-admin";
const otherUserId = "other-member";

test("organization administrator opens only their own credential configuration", { timeout: 90_000 }, async () => {
  const unexpected = [];
  const keyRequests = [];
  const browser = await chromium.launch({
    executablePath: process.env.CHROMIUM_BIN || chromium.executablePath(),
    headless: true,
    args: ["--no-sandbox", "--disable-dev-shm-usage"],
  });
  try {
    const context = await browser.newContext();
    await context.addInitScript(() => {
      sessionStorage.setItem("mwc.api-token", "fixture-organization-admin-token");
      localStorage.setItem("mwc.locale", "en");
      localStorage.setItem("mwc.organization-id", "organization-fixture");
      localStorage.setItem("mwc.view", "workspaces");
    });
    await context.route("**/*", async (route) => {
      const request = route.request();
      const url = new URL(request.url());
      if (url.origin !== baseUrl) return route.abort();
      if (!url.pathname.startsWith("/api/")) return route.continue();
      const method = request.method();
      const path = url.pathname;
      let body;
      if (request.headers().authorization !== "Bearer fixture-organization-admin-token") {
        unexpected.push(`unauthorized ${method} ${path}`);
        return route.fulfill({ status: 401, contentType: "application/json", body: "{}" });
      }
      if (method === "GET" && path === "/api/v1/me") body = {
        user_id: ownUserId, display_name: "Own Admin", system_admin: false,
        memberships: [{ organization_id: organizationId, role: "organization_admin" }],
        api_key_scopes: ["manage_members", "manage_api_keys", "read_workspace"],
        api_key_expires_at: 2_100_000_000, allowed_template_ids: null,
      };
      else if (method === "GET" && path === "/api/v1/organizations") body = { items: [{ id: organizationId, name: "Fixture Organization", created_at: 1_700_000_000 }], next_cursor: null };
      else if (method === "GET" && path === `/api/v1/organizations/${organizationId}/members`) body = {
        items: [
          { user: { id: ownUserId, display_name: "Own Admin", system_admin: false, disabled: false, created_at: 1_700_000_000 }, role: "organization_admin" },
          { user: { id: otherUserId, display_name: "Other Member", system_admin: false, disabled: false, created_at: 1_700_000_000 }, role: "member" },
        ], next_cursor: null,
      };
      else if (method === "GET" && path === "/api/v1/me/api-keys") {
        keyRequests.push(path);
        body = [{ id: "own-key", name: "Own device", prefix: "own-prefix", scopes: ["read_workspace"], created_at: 1_700_000_000, last_used_at: null, expires_at: 2_100_000_000, allowed_template_ids: null, revoked_at: null }];
      }
      else if (method === "GET" && path === `/api/v1/organizations/${organizationId}/quota`) body = null;
      else if (method === "GET" && path === `/api/v1/organizations/${organizationId}/usage-summary`) body = {
        total_count: 0, state_counts: {}, requested: { cpu_millis: 0, memory_mib: 0, gpu_count: 0, disk_gib: 0 },
        temporary_requested_gib: 0, actual: { cpu_millis: null, memory_mib: null, disk_bytes: null, temporary_bytes: null },
        observed_at: null, availability: { cpu: "unknown", memory: "unknown", disk: "unknown", temporary: "unknown" },
        coverage: { total_workspaces: 0, eligible_workspaces: 0, template_label_coverage: "complete" },
      };
      else if (method === "GET" && path === "/api/v1/workspaces") body = { items: [], next_cursor: null, summary: { total_count: 0, requested: { cpu_millis: 0, memory_mib: 0, gpu_count: 0, disk_gib: 0 }, temporary_requested_gib: 0, state_counts: {} } };
      else if (method === "GET" && path === "/api/v1/templates") body = [];
      else if (method === "GET" && path === "/api/v1/node-pools") body = [];
      else if (method === "GET" && path === "/api/v1/webhooks") body = [];
      else if (method === "GET" && (path === `/api/v1/injections/organization/${organizationId}` || path === `/api/v1/injections/user/${ownUserId}`)) body = [];
      else {
        unexpected.push(`${method} ${path}`);
        return route.fulfill({ status: 501, contentType: "application/json", body: "{}" });
      }
      await route.fulfill({ contentType: "application/json", body: JSON.stringify(body) });
    });

    const page = await context.newPage();
    await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
    await page.getByRole("button", { name: "Administration", exact: true }).click();
    await page.getByText("Users and roles", { exact: true }).waitFor();
    const ownRow = page.getByRole("row", { name: /Own Admin/ });
    const otherRow = page.getByRole("row", { name: /Other Member/ });
    assert.equal(await otherRow.getByRole("button", { name: "Credential configuration" }).count(), 0);
    await ownRow.getByRole("button", { name: "Credential configuration" }).click();
    const dialog = page.getByRole("dialog").filter({ hasText: "Own Admin" });
    await dialog.getByText("Own device", { exact: true }).waitFor();
    assert.ok(keyRequests.length > 0);
    assert.equal(await dialog.getByRole("button", { name: "Copy" }).isDisabled(), true);
    assert.deepEqual(unexpected, []);
    await context.close();
  } catch (error) {
    if (unexpected.length) throw new Error(`Unexpected mock API requests: ${unexpected.join(", ")}`, { cause: error });
    throw error;
  } finally {
    await browser.close();
  }
});
