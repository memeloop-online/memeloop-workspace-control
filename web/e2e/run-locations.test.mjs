import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { chromium } from "playwright-core";

const baseUrl = process.env.E2E_BASE_URL ?? "http://127.0.0.1:4173";
const organizationId = "organization-fixture";

test("system administrators manage run locations with idempotent upserts and see delete conflicts", { timeout: 120_000 }, async () => {
  const messages = JSON.parse(await readFile(new URL("../src/locales/en.json", import.meta.url), "utf8"));
  const now = 1_700_000_000;
  const locations = [
    { name: "default", display_name: "Default location", placement: { selector: {}, required_hosts: [], preferred_hosts: [] }, enabled: true, created_at: now, updated_at: now },
    { name: "gpu-east", display_name: "East GPU", placement: { selector: { accelerator: "gpu" }, required_hosts: ["gpu-node-1"], preferred_hosts: ["gpu-node-2"] }, enabled: true, created_at: now, updated_at: now },
    { name: "retired", display_name: "Retired location", placement: { selector: {}, required_hosts: [], preferred_hosts: [] }, enabled: false, created_at: now, updated_at: now },
  ];
  const puts = [];
  const deletes = [];
  const unexpected = [];
  const errors = [];
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_BIN || chromium.executablePath(), headless: true, args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  try {
    const context = await browser.newContext();
    await context.addInitScript(({ organizationId }) => {
      sessionStorage.setItem("mwc.api-token", "fixture-admin-token");
      localStorage.setItem("mwc.locale", "en");
      localStorage.setItem("mwc.organization-id", organizationId);
      localStorage.setItem("mwc.view", "settings");
    }, { organizationId });
    await context.route("**/*", async (route) => {
      const request = route.request();
      const url = new URL(request.url());
      if (url.origin !== new URL(baseUrl).origin) { unexpected.push("external request"); return route.abort(); }
      if (!url.pathname.startsWith("/api/")) return route.continue();
      const path = url.pathname;
      const method = request.method();
      if (request.headers().authorization !== "Bearer fixture-admin-token") {
        unexpected.push(`unauthorized ${method} ${path}`);
        return route.fulfill({ status: 401, contentType: "application/json", body: "{}" });
      }
      const adminPool = /^\/api\/v1\/admin\/node-pools\/([^/]+)$/.exec(path);
      if (method === "PUT" && adminPool) {
        const body = request.postDataJSON();
        const name = decodeURIComponent(adminPool[1]);
        puts.push({ name, body, idempotencyKey: request.headers()["idempotency-key"] ?? null });
        const existing = locations.find((pool) => pool.name === name);
        const updated = { name, display_name: body.display_name, placement: body.placement, enabled: body.enabled, created_at: existing?.created_at ?? now, updated_at: now + puts.length };
        if (existing) Object.assign(existing, updated); else locations.push(updated);
        return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(updated) });
      }
      if (method === "DELETE" && adminPool) {
        const name = decodeURIComponent(adminPool[1]);
        deletes.push(name);
        if (name === "retired") {
          return route.fulfill({ status: 409, contentType: "application/json", body: JSON.stringify({ error: { code: "node_pool_in_use", message: "disable the node pool and remove all template and workspace references before deleting it" } }) });
        }
        const index = locations.findIndex((pool) => pool.name === name);
        if (index >= 0) locations.splice(index, 1);
        return route.fulfill({ status: 204 });
      }
      let body;
      if (method === "GET" && path === "/api/v1/me") body = {
        user_id: "fixture-admin", display_name: "Fixture Admin", system_admin: true,
        memberships: [{ organization_id: organizationId, role: "organization_admin" }],
        api_key_scopes: ["manage_system"], api_key_expires_at: null, allowed_template_ids: null,
      };
      else if (method === "GET" && path === "/api/v1/me/profile") body = { display_name: "Fixture Admin", avatar_url: null };
      else if (method === "GET" && path === "/api/v1/organizations") body = { items: [{ id: organizationId, name: "Fixture Organization", created_at: now }], next_cursor: null };
      else if (method === "GET" && path === "/api/v1/admin/images") body = [];
      else if (method === "GET" && path === "/api/v1/admin/scaling") body = null;
      else if (method === "GET" && path === "/api/v1/admin/node-pools") body = locations;
      else {
        unexpected.push(`${method} ${path}`);
        return route.fulfill({ status: 501, contentType: "application/json", body: "{}" });
      }
      await route.fulfill({ contentType: "application/json", body: JSON.stringify(body) });
    });

    const page = await context.newPage();
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto(`${baseUrl}/#settings`, { waitUntil: "domcontentloaded" });
    const search = page.getByTestId("settings-search");
    await search.fill("Run locations");
    const section = page.getByTestId("settings-section-run-locations");
    await section.waitFor();

    // The list explains each location without exposing a runtime form.
    await section.getByText("East GPU", { exact: true }).waitFor();
    await section.getByText("1 required · 1 preferred · 1 labels", { exact: true }).waitFor();
    const row = (name) => section.getByRole("row", { name: new RegExp(name) });
    assert.equal(await row("Default location").getByRole("button", { name: messages.deleteRunLocation, exact: true }).count(), 0, "the default location is never deletable");
    assert.equal(await row("East GPU").getByRole("button", { name: messages.deleteRunLocation, exact: true }).count(), 0, "an enabled location is not deletable");
    assert.equal(await row("Retired location").getByRole("button", { name: messages.deleteRunLocation, exact: true }).count(), 1);

    // Create with node rules; invalid selector lines are rejected before any request.
    await section.getByRole("button", { name: messages.createRunLocation, exact: true }).click();
    const dialog = page.getByRole("dialog");
    await dialog.getByRole("textbox", { name: messages.runLocationName, exact: true }).fill("gpu-west");
    await dialog.getByRole("textbox", { name: messages.displayName, exact: true }).fill("West GPU");
    await dialog.getByRole("textbox", { name: messages.runLocationRequiredHosts, exact: true }).fill("west-node-1\nwest-node-2");
    await dialog.getByRole("textbox", { name: messages.runLocationPreferredHosts, exact: true }).fill("west-node-3");
    // The label selector lives behind the collapsed advanced section.
    await dialog.locator("summary", { hasText: messages.runLocationSelectorAdvanced }).click();
    await dialog.getByRole("textbox", { name: messages.runLocationSelector, exact: true }).fill("not a pair");
    await dialog.getByRole("button", { name: messages.saveLocation, exact: true }).click();
    await dialog.getByText(messages.runLocationSelectorInvalid, { exact: true }).waitFor();
    assert.equal(puts.length, 0, "invalid selector lines must not reach the API");
    await dialog.getByRole("textbox", { name: messages.runLocationSelector, exact: true }).fill("accelerator=gpu\nzone=west");
    await dialog.getByRole("button", { name: messages.saveLocation, exact: true }).click();
    await dialog.waitFor({ state: "hidden" });
    assert.deepEqual(puts.map(({ name }) => name), ["gpu-west"]);
    assert.deepEqual(puts[0].body, {
      display_name: "West GPU",
      enabled: true,
      placement: { selector: { accelerator: "gpu", zone: "west" }, required_hosts: ["west-node-1", "west-node-2"], preferred_hosts: ["west-node-3"] },
    });
    assert.ok(puts[0].idempotencyKey, "the upsert carries an Idempotency-Key");
    await row("West GPU").waitFor();

    // Edit keeps the stable ID and updates the human name.
    await row("West GPU").getByRole("button", { name: messages.editRunLocation, exact: true }).click();
    const editDialog = page.getByRole("dialog");
    assert.equal(await editDialog.getByRole("textbox", { name: messages.runLocationName, exact: true }).isDisabled(), true);
    await editDialog.getByRole("textbox", { name: messages.displayName, exact: true }).fill("West GPU prime");
    await editDialog.getByRole("button", { name: messages.saveLocation, exact: true }).click();
    await editDialog.waitFor({ state: "hidden" });
    assert.equal(puts.length, 2);
    assert.equal(puts[1].name, "gpu-west");
    assert.equal(puts[1].body.display_name, "West GPU prime");
    assert.ok(puts[1].idempotencyKey);
    await row("West GPU prime").waitFor();

    // Disable, then delete with confirmation.
    await row("West GPU prime").getByRole("button", { name: messages.pluginDisable, exact: true }).click();
    await row("West GPU prime").getByRole("button", { name: messages.deleteRunLocation, exact: true }).waitFor();
    assert.equal(puts.length, 3);
    assert.equal(puts[2].body.enabled, false);
    await row("West GPU prime").getByRole("button", { name: messages.deleteRunLocation, exact: true }).click();
    const confirm = page.getByRole("alertdialog");
    await confirm.getByRole("button", { name: messages.deleteRunLocation, exact: true }).click();
    await confirm.waitFor({ state: "hidden" });
    assert.deepEqual(deletes, ["gpu-west"]);
    assert.equal(await row("West GPU prime").count(), 0);

    // A referenced location reports the backend conflict instead of vanishing.
    await row("Retired location").getByRole("button", { name: messages.deleteRunLocation, exact: true }).click();
    const conflictConfirm = page.getByRole("alertdialog");
    await conflictConfirm.getByRole("button", { name: messages.deleteRunLocation, exact: true }).click();
    await page.getByText("disable the node pool and remove all template and workspace references before deleting it", { exact: true }).waitFor();
    // The footer Cancel carries visible text; the title dismiss shares its aria-label.
    await conflictConfirm.getByText(messages.cancel, { exact: true }).click();
    assert.deepEqual(deletes, ["gpu-west", "retired"]);
    assert.equal(await row("Retired location").count(), 1, "a conflicted delete keeps the location listed");

    assert.deepEqual(unexpected, [], "unexpected API requests");
    assert.deepEqual(errors, [], "browser errors");
    await context.close();
  } finally { await browser.close(); }
});

test("template-restricted system administrators never see or request run locations", { timeout: 60_000 }, async () => {
  const messages = JSON.parse(await readFile(new URL("../src/locales/en.json", import.meta.url), "utf8"));
  const now = 1_700_000_000;
  const unexpected = [];
  const errors = [];
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_BIN || chromium.executablePath(), headless: true, args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  try {
    const context = await browser.newContext();
    await context.addInitScript(({ organizationId }) => {
      sessionStorage.setItem("mwc.api-token", "fixture-restricted-token");
      localStorage.setItem("mwc.locale", "en");
      localStorage.setItem("mwc.organization-id", organizationId);
      localStorage.setItem("mwc.view", "settings");
    }, { organizationId });
    await context.route("**/*", async (route) => {
      const request = route.request();
      const url = new URL(request.url());
      if (url.origin !== new URL(baseUrl).origin) { unexpected.push("external request"); return route.abort(); }
      if (!url.pathname.startsWith("/api/")) return route.continue();
      const path = url.pathname;
      const method = request.method();
      // /api/v1/admin/node-pools deliberately has no fixture: any request for it
      // proves the section leaked past the template-restriction guard.
      let body;
      if (method === "GET" && path === "/api/v1/me") body = {
        user_id: "fixture-restricted", display_name: "Restricted Admin", system_admin: true,
        memberships: [{ organization_id: organizationId, role: "organization_admin" }],
        api_key_scopes: ["manage_system"], api_key_expires_at: null, allowed_template_ids: ["template-1"],
      };
      else if (method === "GET" && path === "/api/v1/me/profile") body = { display_name: "Restricted Admin", avatar_url: null };
      else if (method === "GET" && path === "/api/v1/organizations") body = { items: [{ id: organizationId, name: "Fixture Organization", created_at: now }], next_cursor: null };
      else if (method === "GET" && path === "/api/v1/admin/images") body = [];
      else if (method === "GET" && path === "/api/v1/admin/scaling") body = null;
      else {
        unexpected.push(`${method} ${path}`);
        return route.fulfill({ status: 501, contentType: "application/json", body: "{}" });
      }
      await route.fulfill({ contentType: "application/json", body: JSON.stringify(body) });
    });

    const page = await context.newPage();
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto(`${baseUrl}/#settings`, { waitUntil: "domcontentloaded" });
    await page.getByTestId("settings-section-images").waitFor();
    assert.equal(await page.getByTestId("settings-section-run-locations").count(), 0);
    const search = page.getByTestId("settings-search");
    await search.fill("Run locations");
    await page.getByText("No matching settings.", { exact: true }).waitFor();
    assert.equal(await page.getByTestId("settings-section-run-locations").count(), 0);
    assert.deepEqual(unexpected, [], "no administrator node-pool requests may fire");
    assert.deepEqual(errors, [], "browser errors");
    await context.close();
  } finally { await browser.close(); }
});
