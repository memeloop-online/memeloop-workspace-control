import assert from "node:assert/strict";
import { test } from "node:test";
import { chromium } from "playwright-core";

const baseUrl = process.env.E2E_BASE_URL ?? "http://127.0.0.1:4173";
const organizationId = "organization-fixture";
const aliceId = "user-alice";
const bobId = "user-bob";
const templateId = "template-fixture";
const expiresAt = 2_100_000_000;

function apiKey(id, name, token) {
  return {
    id,
    name,
    token,
    prefix: `${id}-prefix`,
    scopes: ["read_workspace"],
    created_at: 1_700_000_000,
    last_used_at: null,
    expires_at: expiresAt,
    allowed_template_ids: null,
    revoked_at: null,
  };
}

test("administration selects only the clicked user's API keys and manages their UI", { timeout: 90_000 }, async () => {
  const keys = new Map([
    [aliceId, [apiKey("alice-existing", "Alice device", "fixture-alice-token")]],
    [bobId, [apiKey("bob-existing", "Bob automation", "fixture-bob-token")]],
  ]);
  const keyReads = [];
  const keyReadbacks = [];
  const writes = [];
  const unexpected = [];
  const adminReads = [];
  let principalScopes = ["manage_system", "manage_organization", "manage_members", "manage_api_keys", "read_workspace"];
  let principalAllowedTemplateIds = null;
  const browser = await chromium.launch({
    executablePath: process.env.CHROMIUM_BIN || chromium.executablePath(),
    headless: true,
    args: ["--no-sandbox", "--disable-dev-shm-usage"],
  });

  try {
    const context = await browser.newContext({ permissions: ["clipboard-read", "clipboard-write"] });
    await context.addInitScript(() => {
      sessionStorage.setItem("mwc.api-token", "fixture-admin-token");
      localStorage.setItem("mwc.locale", "en");
      localStorage.setItem("mwc.organization-id", "organization-fixture");
      localStorage.setItem("mwc.view", "workspaces");
    });
    await context.route("**/*", async (route) => {
      const request = route.request();
      const url = new URL(request.url());
      if (url.origin !== baseUrl) {
        unexpected.push(`${request.method()} ${url.href}`);
        return route.abort();
      }
      if (!url.pathname.startsWith("/api/")) return route.continue();

      const method = request.method();
      const path = url.pathname;
      if (method === "GET" && path.startsWith("/api/v1/admin/")) adminReads.push(path);
      let response;
      if (request.headers().authorization !== "Bearer fixture-admin-token") {
        unexpected.push(`unauthorized fixture request: ${method} ${path}`);
        response = { status: 401, body: { error: { message: "Missing fixture token" } } };
      } else if (method === "GET" && path === "/api/v1/me") {
        response = { body: {
          user_id: "fixture-admin", display_name: "Fixture Admin", system_admin: true,
          memberships: [{ organization_id: organizationId, role: "organization_admin" }],
          api_key_scopes: principalScopes,
          api_key_expires_at: expiresAt, allowed_template_ids: principalAllowedTemplateIds,
        } };
      } else if (method === "GET" && path === "/api/v1/organizations") {
        response = { body: { items: [{ id: organizationId, name: "Fixture Organization", created_at: 1_700_000_000 }], next_cursor: null } };
      } else if (method === "GET" && path === "/api/v1/me/api-keys") {
        response = { body: [] };
      } else if (method === "GET" && path === "/api/v1/organizations/organization-fixture/quota") {
        response = { body: null };
      } else if (method === "GET" && path === "/api/v1/organizations/organization-fixture/usage-summary") {
        response = { body: {
          total_count: 0,
          state_counts: {},
          requested: { cpu_millis: 0, memory_mib: 0, gpu_count: 0, disk_gib: 0 },
          temporary_requested_gib: 0,
          actual: { cpu_millis: null, memory_mib: null, disk_bytes: null, temporary_bytes: null },
          observed_at: null,
          availability: { cpu: "unknown", memory: "unknown", disk: "unknown", temporary: "unknown" },
          coverage: { total_workspaces: 0, eligible_workspaces: 0, template_label_coverage: "complete" },
        } };
      } else if (method === "GET" && (path === "/api/v1/injections/organization/organization-fixture" || path === "/api/v1/injections/user/fixture-admin")) {
        response = { body: [] };
      } else if (method === "GET" && path === "/api/v1/templates") {
        response = { body: [{ id: templateId, name: "Fixture template", organization_id: organizationId, enabled: true }] };
      } else if (method === "GET" && path === "/api/v1/webhooks") {
        response = { body: [] };
      } else if (method === "GET" && path === "/api/v1/node-pools") {
        response = { body: [] };
      } else if (method === "GET" && path === "/api/v1/admin/images") {
        response = { body: [] };
      } else if (method === "GET" && path === "/api/v1/admin/scaling") {
        response = { body: null };
      } else if (method === "GET" && path === "/api/v1/workspaces") {
        response = { body: { items: [], next_cursor: null, summary: { total_count: 0, requested: { cpu_millis: 0, memory_mib: 0, gpu_count: 0, disk_gib: 0 }, temporary_requested_gib: 0, state_counts: {} } } };
      } else if (method === "GET" && path === "/api/v1/admin/users") {
        response = { body: { items: [
          { id: aliceId, display_name: "Alice Fixture", system_admin: false, disabled: false, created_at: 1_700_000_000, membership_role: "member" },
          { id: bobId, display_name: "Bob Fixture", system_admin: false, disabled: false, created_at: 1_700_000_000, membership_role: "member" },
        ], next_cursor: null } };
      } else {
        const match = /^\/api\/v1\/admin\/users\/(user-alice|user-bob)\/api-keys(?:\/([^/]+))?$/.exec(path);
        if (match && method === "GET" && !match[2]) {
          keyReads.push(match[1]);
          keyReadbacks.push({
            userId: match[1],
            items: keys.get(match[1]).map(({ id, token, scopes, allowed_template_ids, revoked_at }) => ({
              id, token, scopes, allowed_template_ids, revoked_at,
            })),
          });
          response = { body: { items: keys.get(match[1]), next_cursor: null } };
        } else if (match && method === "POST" && !match[2]) {
          const input = request.postDataJSON();
          writes.push({ method, userId: match[1], input });
          const created = { ...apiKey("alice-created", input.name, "fixture-created-token"), scopes: input.scopes, expires_at: input.expires_at, allowed_template_ids: input.allowed_template_ids };
          keys.get(match[1]).push(created);
          response = { body: created };
        } else if (match && method === "DELETE" && match[2]) {
          writes.push({ method, userId: match[1], keyId: match[2], input: request.postDataJSON() });
          const key = keys.get(match[1]).find((item) => item.id === match[2]);
          if (key) key.revoked_at = Math.floor(Date.now() / 1_000);
          response = { status: 204 };
        }
      }

      if (!response) {
        unexpected.push(`${method} ${url.pathname}${url.search}`);
        response = { status: 501, body: { error: { message: "Unmocked fixture endpoint" } } };
      }
      await route.fulfill({ status: response.status ?? 200, contentType: "application/json", body: response.status === 204 ? "" : JSON.stringify(response.body) });
    });

    const page = await context.newPage();
    await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
    await page.getByRole("button", { name: "Administration", exact: true }).click();
    await page.getByText("Users and roles", { exact: true }).waitFor();

    const aliceRow = page.getByRole("row", { name: /Alice Fixture/ });
    const bobRow = page.getByRole("row", { name: /Bob Fixture/ });
    const templatesLoaded = page.waitForResponse((response) => {
      const url = new URL(response.url());
      return url.pathname === "/api/v1/templates" && url.searchParams.get("organization_id") === organizationId;
    });
    await aliceRow.getByRole("button", { name: "Credential configuration" }).click();
    const editor = page.getByRole("dialog").filter({ hasText: "Alice Fixture" });
    await editor.getByText("Credential configuration · Alice Fixture", { exact: true }).waitFor();
    assert.deepEqual((await (await templatesLoaded).json()).map((template) => template.id), [templateId]);
    assert.equal(await editor.getByRole("tab").count(), 0);
    await editor.getByText("Alice device", { exact: true }).waitFor();
    assert.deepEqual([...new Set(keyReads)], [aliceId]);
    assert.equal(await editor.getByText("Bob automation", { exact: true }).count(), 0);

    await editor.getByRole("button", { name: "Copy" }).click();
    assert.equal(await page.evaluate(() => navigator.clipboard.readText()), "fixture-alice-token");
    await editor.getByRole("button", { name: "Create API key" }).first().click();
    await editor.getByRole("textbox", { name: "Key name" }).fill("Alice CI key");
    await editor.getByRole("checkbox", { name: "Manage API keys" }).check();
    await editor.getByRole("checkbox", { name: "Only allow selected templates" }).check();
    const templatePicker = editor.getByRole("combobox", { name: "Workspace templates" });
    await templatePicker.fill("Fixture template");
    await templatePicker.press("ArrowDown");
    await templatePicker.press("Enter");
    await editor.getByRole("button", { name: "Remove template Fixture template" }).waitFor();
    await editor.getByRole("button", { name: "Create API key" }).last().click();
    await editor.getByText("Alice CI key", { exact: true }).waitFor();
    assert.deepEqual(writes[0], {
      method: "POST", userId: aliceId,
      input: { name: "Alice CI key", scopes: ["read_workspace", "manage_api_keys"], expires_at: writes[0].input.expires_at, allowed_template_ids: [templateId] },
    });
    assert.ok(writes[0].input.expires_at > Math.floor(Date.now() / 1_000));
    assert.deepEqual(keyReadbacks.at(-1), {
      userId: aliceId,
      items: [
        { id: "alice-existing", token: "fixture-alice-token", scopes: ["read_workspace"], allowed_template_ids: null, revoked_at: null },
        { id: "alice-created", token: "fixture-created-token", scopes: ["read_workspace", "manage_api_keys"], allowed_template_ids: [templateId], revoked_at: null },
      ],
    });
    assert.equal(await editor.getByText("fixture-created-token", { exact: true }).count(), 1);

    const createdRow = editor.getByRole("row", { name: /Alice CI key/ });
    await createdRow.getByRole("button", { name: "Revoke" }).click();
    await page.getByRole("textbox", { name: "Reason for revocation" }).fill("CI fixture revoke");
    await page.getByRole("button", { name: "Revoke", exact: true }).last().click();
    await editor.getByRole("row", { name: /Alice CI key/ }).getByText("Revoked", { exact: true }).waitFor();
    assert.deepEqual(writes[1], { method: "DELETE", userId: aliceId, keyId: "alice-created", input: { reason: "CI fixture revoke" } });

    await editor.getByRole("button", { name: "Close" }).click();
    await aliceRow.getByRole("button", { name: "Credential configuration" }).click();
    const reopened = page.getByRole("dialog").filter({ hasText: "Alice Fixture" });
    await reopened.getByRole("row", { name: /Alice CI key/ }).getByText("Revoked", { exact: true }).waitFor();
    assert.equal(keyReadbacks.at(-1).userId, aliceId);
    assert.notEqual(keyReadbacks.at(-1).items.find((item) => item.id === "alice-created").revoked_at, null);
    await reopened.getByRole("button", { name: "Close" }).click();
    await bobRow.getByRole("button", { name: "Credential configuration" }).click();
    const bobEditor = page.getByRole("dialog").filter({ hasText: "Bob Fixture" });
    await bobEditor.getByText("Bob automation", { exact: true }).waitFor();
    assert.equal(await bobEditor.getByText("Alice device", { exact: true }).count(), 0);
    assert.equal(await bobEditor.getByText("Alice CI key", { exact: true }).count(), 0);
    assert.equal(keyReadbacks.at(-1).userId, bobId);
    assert.deepEqual(keyReadbacks.at(-1).items.map((item) => item.id), ["bob-existing"]);
    await bobEditor.getByRole("button", { name: "Close" }).click();

    principalScopes = ["manage_system", "manage_api_keys", "read_workspace"];
    principalAllowedTemplateIds = [templateId];
    const adminReadsBeforeRestrictedLogin = adminReads.length;
    await page.reload({ waitUntil: "domcontentloaded" });
    await page.getByRole("button", { name: "Administration", exact: true }).click();
    await page.getByText("Credential configuration", { exact: true }).waitFor();
    assert.equal(await page.getByText("Users and roles", { exact: true }).count(), 0);
    assert.equal(adminReads.length, adminReadsBeforeRestrictedLogin);
    assert.deepEqual(unexpected, []);
    await context.close();
  } catch (error) {
    if (unexpected.length) throw new Error(`Unexpected mock API requests: ${unexpected.join(", ")}`, { cause: error });
    throw error;
  } finally {
    await browser.close();
  }
});
