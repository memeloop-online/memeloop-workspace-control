import assert from "node:assert/strict";
import { test } from "node:test";
import { chromium } from "playwright-core";

const baseUrl = process.env.E2E_BASE_URL ?? "http://127.0.0.1:4173";
const organizationId = "organization-fixture";

test("ordinary member Settings cannot reveal administrator sections or request administrator APIs", { timeout: 90_000 }, async () => {
  const unexpected = [];
  const browser = await chromium.launch({
    executablePath: process.env.CHROMIUM_BIN || chromium.executablePath(),
    headless: true,
    args: ["--no-sandbox", "--disable-dev-shm-usage"],
  });
  try {
    const context = await browser.newContext();
    await context.addInitScript(() => {
      sessionStorage.setItem("mwc.api-token", "fixture-member-token");
      localStorage.setItem("mwc.locale", "en");
      localStorage.setItem("mwc.organization-id", "organization-fixture");
      localStorage.setItem("mwc.view", "workspaces");
    });
    await context.route("**/*", async (route) => {
      const request = route.request();
      const url = new URL(request.url());
      if (url.origin !== baseUrl) return route.abort();
      if (!url.pathname.startsWith("/api/")) return route.continue();
      const path = url.pathname;
      const method = request.method();
      let body;
      if (request.headers().authorization !== "Bearer fixture-member-token") {
        unexpected.push(`unauthorized ${method} ${path}`);
        return route.fulfill({ status: 401, contentType: "application/json", body: "{}" });
      }
      if (method === "GET" && path === "/api/v1/me") body = {
        user_id: "ordinary-member", display_name: "Ordinary Member", system_admin: false,
        memberships: [{ organization_id: organizationId, role: "member" }],
        api_key_scopes: ["read_workspace"], api_key_expires_at: 2_100_000_000, allowed_template_ids: null,
      };
      else if (method === "GET" && path === "/api/v1/me/profile") body = { display_name: "Ordinary Member", avatar_url: null };
      else if (method === "GET" && path === "/api/v1/organizations") body = { items: [{ id: organizationId, name: "Fixture Organization", created_at: 1_700_000_000 }], next_cursor: null };
      else if (method === "GET" && path === "/api/v1/workspaces") body = { items: [], next_cursor: null, summary: { total_count: 0, requested: { cpu_millis: 0, memory_mib: 0, gpu_count: 0, disk_gib: 0 }, temporary_requested_gib: 0, state_counts: {} } };
      else if (method === "GET" && path === "/api/v1/templates") body = [];
      else if (method === "GET" && path === "/api/v1/node-pools") body = [];
      else {
        unexpected.push(`${method} ${path}`);
        return route.fulfill({ status: 501, contentType: "application/json", body: "{}" });
      }
      await route.fulfill({ contentType: "application/json", body: JSON.stringify(body) });
    });

    const page = await context.newPage();
    await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await page.getByRole("textbox", { name: "Display name" }).waitFor();
    const search = page.getByTestId("settings-search");
    await search.fill("usersRoles");
    assert.equal(await page.getByText("Users and roles", { exact: true }).count(), 0);
    assert.equal(await page.getByText("System", { exact: true }).count(), 0);
    assert.equal(await page.getByRole("button", { name: "Credential configuration" }).count(), 0);
    await search.fill("Display name");
    await page.getByRole("textbox", { name: "Display name" }).waitFor();
    assert.deepEqual(unexpected, []);
    await context.close();
  } catch (error) {
    if (unexpected.length) throw new Error(`Unexpected mock API requests: ${unexpected.join(", ")}`, { cause: error });
    throw error;
  } finally {
    await browser.close();
  }
});

test("personal API-key settings remain available without a selected organization", { timeout: 90_000 }, async () => {
  const unexpected = [];
  const browser = await chromium.launch({
    executablePath: process.env.CHROMIUM_BIN || chromium.executablePath(),
    headless: true,
    args: ["--no-sandbox", "--disable-dev-shm-usage"],
  });
  try {
    const context = await browser.newContext();
    await context.addInitScript(() => {
      sessionStorage.setItem("mwc.api-token", "fixture-no-organization-token");
      localStorage.setItem("mwc.locale", "en");
      localStorage.removeItem("mwc.organization-id");
      localStorage.setItem("mwc.view", "settings");
    });
    await context.route("**/*", async (route) => {
      const request = route.request();
      const url = new URL(request.url());
      if (url.origin !== baseUrl) return route.abort();
      if (!url.pathname.startsWith("/api/")) return route.continue();
      const path = url.pathname;
      let body;
      if (request.headers().authorization !== "Bearer fixture-no-organization-token") {
        unexpected.push(`unauthorized ${request.method()} ${path}`);
        return route.fulfill({ status: 401, contentType: "application/json", body: "{}" });
      }
      if (request.method() === "GET" && path === "/api/v1/me") body = {
        user_id: "unscoped-user", display_name: "Unscoped User", system_admin: false, memberships: [],
        api_key_scopes: ["manage_api_keys", "read_workspace"], api_key_expires_at: null, allowed_template_ids: null,
      };
      else if (request.method() === "GET" && path === "/api/v1/me/profile") body = { display_name: "Unscoped User", avatar_url: null };
      else if (request.method() === "GET" && path === "/api/v1/organizations") body = { items: [], next_cursor: null };
      else if (request.method() === "GET" && path === "/api/v1/me/api-keys") body = [];
      else {
        unexpected.push(`${request.method()} ${path}`);
        return route.fulfill({ status: 501, contentType: "application/json", body: "{}" });
      }
      await route.fulfill({ contentType: "application/json", body: JSON.stringify(body) });
    });

    const page = await context.newPage();
    await page.goto(`${baseUrl}/#settings`, { waitUntil: "domcontentloaded" });
    const search = page.getByTestId("settings-search");
    await search.fill("api-keys");
    await page.getByTestId("settings-section-api-keys").waitFor();
    assert.equal(await page.getByTestId("settings-section-users").count(), 0);
    assert.deepEqual(unexpected, []);
    await context.close();
  } catch (error) {
    if (unexpected.length) throw new Error(`Unexpected mock API requests: ${unexpected.join(", ")}`, { cause: error });
    throw error;
  } finally {
    await browser.close();
  }
});
