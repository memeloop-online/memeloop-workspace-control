import assert from "node:assert/strict";
import { mkdir, readFile } from "node:fs/promises";
import { test } from "node:test";
import { chromium } from "playwright-core";

const baseUrl = process.env.E2E_BASE_URL ?? "http://127.0.0.1:4173";
const organizationId = "editing-organization";
const messages = JSON.parse(await readFile(new URL("../src/locales/en.json", import.meta.url), "utf8"));
const resources = { cpu_millis: 2000, memory_mib: 4096, gpu_count: 0, disk_gib: 20 };
function item(id, state) {
  return { workspace: {
    id, short_id: id, name: id, state, resources: { ...resources },
    organization_id: organizationId, owner_id: "fixture-user", template_id: "fixture-template",
    node_pool: "default", image: "example.invalid/dev:latest", access_mode: "internal",
    pod_requests: { cpu_millis: 100, memory_mib: 128 }, workspace_user: "user", workspace_home: "/home/user",
    buildkit: false, storage_policy: { temporary_storage_gib: 10, use_longhorn: false }, cluster_access: false,
    egress_policy: "internet_only", runtime_class_name: null,
    placement: { allowed_node_pools: ["default"], default_node_pool: "default" },
    generation: 1, created_at: 1_700_000_000, updated_at: 1_700_000_000,
  }, namespace: "fixture", ssh_connection: null, ssh_host: null, ssh_port: null, ssh_command: null,
  ssh_config: null, web_shell_url: null, injection_sources: [], workspace_host_key: null, jump_host_key: null };
}

test("workspace settings and mapping editing retain drafts, stable addresses and current generations", { timeout: 120_000 }, async () => {
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_BIN || chromium.executablePath(), headless: true, args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  const workspaces = [item("ready-workspace", "ready"), item("stopped-workspace", "stopped")];
  const mappings = [{ id: "mapping-one", internal_port: 3000, display_name: "Application", subdomain: "mapping-one", status: "stopped", managed: false, purpose: "user", https_url: "https://p-mapping-one.example.test" }];
  const writes = [];
  const unexpected = [];
  const errors = [];
  let failNextSettings = true;
  try {
    const context = await browser.newContext();
    await context.addInitScript(({ organizationId }) => {
      sessionStorage.setItem("mwc.api-token", "editing-fixture-token");
      localStorage.setItem("mwc.locale", "en");
      localStorage.setItem("mwc.organization-id", organizationId);
      localStorage.setItem("mwc.view", "workspaces");
    }, { organizationId });
    await context.route("**/*", async (route) => {
      const request = route.request();
      const url = new URL(request.url());
      if (url.origin !== new URL(baseUrl).origin) { unexpected.push("external request"); return route.abort(); }
      if (!url.pathname.startsWith("/api/")) return route.continue();
      const path = url.pathname;
      let body;
      let status = 200;
      if (request.method() === "GET") {
        if (path === "/api/v1/me") body = { user_id: "fixture-user", display_name: "Fixture", system_admin: true, memberships: [{ organization_id: organizationId, role: "organization_admin" }], api_key_scopes: ["read_workspace", "connect_workspace", "change_workspace_state", "manage_system"], api_key_expires_at: 2_100_000_000, allowed_template_ids: null };
        else if (path === "/api/v1/me/profile") body = { display_name: "Fixture", avatar_url: null };
        else if (path === "/api/v1/organizations") body = { items: [{ id: organizationId, name: "Editing", created_at: 1_700_000_000 }], next_cursor: null };
        else if (path === "/api/v1/workspaces") body = { items: workspaces, next_cursor: null };
        else if (path === "/api/v1/workspace-runtimes") body = [];
        else if (path === `/api/v1/organizations/${organizationId}/quota`) body = null;
        else if (path === `/api/v1/organizations/${organizationId}/usage-summary`) body = { total_count: 2, state_counts: { ready: 1, stopped: 1 }, requested: resources, temporary_requested_gib: 20, actual: { cpu_millis: null, memory_mib: null, disk_bytes: null, temporary_bytes: null }, observed_at: null, availability: { cpu: "unknown", memory: "unknown", disk: "unknown", temporary: "unknown" }, coverage: { total_workspaces: 2, eligible_workspaces: 1, template_label_coverage: "complete" } };
        else if (path === "/api/v1/templates") body = [{ id: "fixture-template", name: "Fixture", enabled: true }];
        else if (path === "/api/v1/node-pools") body = [{ name: "default", display_name: "Default" }];
        else if (path.endsWith("/port-mappings")) body = mappings;
        else if (path.startsWith("/api/v1/injections/") || path === "/api/v1/me/api-keys") body = [];
      } else if (request.method() === "POST") {
        const input = request.postDataJSON();
        writes.push({ path, input });
        assert.ok(request.headers()["idempotency-key"]);
        if (path.endsWith("/port-mappings")) {
          const label = input.subdomain ?? "1a2b3c4d";
          body = { id: "mapping-created", internal_port: input.internal_port, display_name: input.display_name ?? null, subdomain: label, status: "ready", managed: false, purpose: "user", https_url: `https://p-${label}.example.test` };
          mappings.push(body);
        }
      } else if (request.method() === "PUT") {
        const input = request.postDataJSON();
        writes.push({ path, input });
        assert.ok(request.headers()["idempotency-key"]);
        const match = /^\/api\/v1\/workspaces\/([^/]+)\/(settings|temporary-storage)$/.exec(path);
        if (match) {
          const target = workspaces.find(({ workspace }) => workspace.id === match[1]);
          if (match[2] === "settings" && failNextSettings) {
            failNextSettings = false;
            status = 409; body = { error: { code: "workspace_settings_update_conflict", message: "stale workspace" } };
          } else {
            assert.equal(input.expected_generation, target.workspace.generation);
            if (match[2] === "settings") {
              target.workspace.name = input.name;
              Object.assign(target.workspace.resources, { cpu_millis: input.cpu_millis, memory_mib: input.memory_mib, gpu_count: input.gpu_count });
            } else target.workspace.storage_policy.temporary_storage_gib = input.temporary_storage_gib;
            target.workspace.generation += 1;
            body = target;
          }
        } else if (path.endsWith("/port-mappings/mapping-one")) {
          if (input.subdomain === "taken-label") {
            status = 409; body = { error: { code: "subdomain_conflict", message: "label already taken" } };
          } else {
            const label = input.subdomain ?? mappings[0].subdomain;
            Object.assign(mappings[0], input, { subdomain: label, https_url: `https://p-${label}.example.test` });
            body = mappings[0];
          }
        }
      }
      if (body === undefined) { unexpected.push(`${request.method()} ${path}`); status = 501; }
      await route.fulfill({ status, contentType: "application/json", body: JSON.stringify(body ?? null) });
    });
    const page = await context.newPage();
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
    await page.getByRole("group", { name: "ready-workspace", exact: true }).getByRole("button", { name: messages.editParameters, exact: true }).click();
    let dialog = page.getByRole("dialog");
    assert.equal(await dialog.getByLabel(messages.workspaceCpuMillis, { exact: true }).isDisabled(), true);
    await dialog.getByLabel(messages.workspaceName, { exact: true }).fill("支付工作区");
    await dialog.getByRole("button", { name: messages.saveSettings, exact: true }).click();
    await dialog.getByRole("alert").waitFor();
    assert.equal(await dialog.getByLabel(messages.workspaceName, { exact: true }).inputValue(), "支付工作区");
    await Promise.all([page.waitForResponse((response) => response.url().endsWith("/settings") && response.status() === 200), dialog.getByRole("button", { name: messages.saveSettings, exact: true }).click()]);
    await dialog.getByRole("button", { name: messages.cancel, exact: true }).last().click();
    await page.getByRole("group", { name: "支付工作区", exact: true }).waitFor();

    await page.getByRole("group", { name: "stopped-workspace", exact: true }).getByRole("button", { name: messages.editParameters, exact: true }).click();
    dialog = page.getByRole("dialog");
    await dialog.getByLabel(messages.workspaceCpuMillis, { exact: true }).fill("3000");
    await Promise.all([page.waitForResponse((response) => response.url().endsWith("/settings") && response.status() === 200), dialog.getByRole("button", { name: messages.saveSettings, exact: true }).click()]);
    await dialog.getByLabel(messages.workspaceTemporaryStorageGiB, { exact: true }).fill("12");
    await dialog.getByRole("button", { name: messages.saveTemporaryStorage, exact: true }).click();
    await dialog.getByText(messages.settingsStorageSaved, { exact: true }).waitFor();
    assert.equal(workspaces[1].workspace.state, "stopped");
    assert.equal(workspaces[1].workspace.resources.cpu_millis, 3000);
    await dialog.getByRole("button", { name: messages.cancel, exact: true }).last().click();

    await page.getByRole("group", { name: "stopped-workspace", exact: true }).getByRole("button", { name: messages.portMappings, exact: true }).click();
    dialog = page.getByRole("dialog");
    await dialog.getByText("MWC_PORT_MAPPINGS_FILE", { exact: true }).waitFor();
    await dialog.getByText("MWC_PORT_MAPPINGS_JSON", { exact: true }).waitFor();
    await dialog.getByText('cat "$MWC_PORT_MAPPINGS_FILE"', { exact: true }).waitFor();
    await dialog.getByRole("button", { name: messages.edit, exact: true }).click();
    const name = dialog.getByLabel(messages.displayNameOptional, { exact: true });
    const subdomain = dialog.getByLabel(messages.portMappingSubdomainOptional, { exact: true });
    assert.equal(await subdomain.inputValue(), "mapping-one");
    await name.fill("名".repeat(81));
    assert.equal(await dialog.getByRole("button", { name: messages.saveMapping, exact: true }).isDisabled(), true);
    await name.fill("名".repeat(80));
    await dialog.getByRole("button", { name: messages.saveMapping, exact: true }).click();
    await dialog.getByText("名".repeat(80), { exact: true }).waitFor();
    await dialog.getByRole("button", { name: messages.edit, exact: true }).click();
    await dialog.getByLabel(messages.internalPort, { exact: true }).fill("4000");
    await name.fill("");
    await subdomain.fill("");
    await dialog.getByRole("button", { name: messages.saveMapping, exact: true }).click();
    await dialog.getByText("4000", { exact: true }).waitFor();
    assert.equal(mappings[0].id, "mapping-one");
    assert.equal(mappings[0].https_url, "https://p-mapping-one.example.test");
    assert.equal(mappings[0].display_name, null);
    const mappingWrites = writes.filter(({ path }) => path.endsWith("/port-mappings/mapping-one"));
    assert.equal(mappingWrites[0].input.subdomain, "mapping-one");
    assert.equal("subdomain" in mappingWrites[mappingWrites.length - 1].input, false);
    assert.equal(mappings[0].subdomain, "mapping-one");
    assert.equal(await dialog.getByRole("button", { name: messages.open, exact: true }).isDisabled(), true);

    await dialog.getByRole("button", { name: messages.edit, exact: true }).click();
    await subdomain.fill("taken-label");
    await dialog.getByRole("button", { name: messages.saveMapping, exact: true }).click();
    await dialog.getByText(messages.portMappingSubdomainConflict, { exact: true }).waitFor();
    assert.equal(await subdomain.inputValue(), "taken-label");
    await subdomain.fill("  Neat-Label  ");
    await Promise.all([page.waitForResponse((response) => response.url().endsWith("/port-mappings/mapping-one") && response.status() === 200), dialog.getByRole("button", { name: messages.saveMapping, exact: true }).click()]);
    await dialog.getByText("https://p-neat-label.example.test", { exact: true }).waitFor();
    const labelWrites = writes.filter(({ path }) => path.endsWith("/port-mappings/mapping-one"));
    assert.equal(labelWrites[labelWrites.length - 1].input.subdomain, "neat-label");
    assert.equal(mappings[0].https_url, "https://p-neat-label.example.test");
    assert.equal(await dialog.getByRole("button", { name: messages.open, exact: true }).isDisabled(), true);
    if (process.env.RUNNER_TEMP) {
      const screenshotDir = `${process.env.RUNNER_TEMP}/mwc-settings-screenshots`;
      await mkdir(screenshotDir, { recursive: true });
      await dialog.screenshot({ path: `${screenshotDir}/port-mapping-domains.png` });
    }

    await dialog.getByRole("button", { name: messages.edit, exact: true }).click();
    await subdomain.fill("bad_label");
    await dialog.getByText(messages.portMappingSubdomainInvalid, { exact: true }).waitFor();
    assert.equal(await dialog.getByRole("button", { name: messages.saveMapping, exact: true }).isDisabled(), true);
    const beforeInvalidWrites = writes.length;
    await subdomain.press("Enter");
    assert.equal(writes.length, beforeInvalidWrites);
    await dialog.getByRole("button", { name: messages.close, exact: true }).click();

    await page.getByRole("group", { name: "支付工作区", exact: true }).getByRole("button", { name: messages.portMappings, exact: true }).click();
    dialog = page.getByRole("dialog");
    await dialog.getByText('cat "$MWC_PORT_MAPPINGS_FILE"', { exact: true }).waitFor();
    await dialog.getByLabel(messages.internalPort, { exact: true }).fill("5000");
    await Promise.all([page.waitForResponse((response) => response.url().endsWith("/port-mappings") && response.request().method() === "POST" && response.status() === 200), dialog.getByRole("button", { name: messages.addMapping, exact: true }).click()]);
    await dialog.getByText("https://p-1a2b3c4d.example.test", { exact: true }).waitFor();
    const createWrite = writes[writes.length - 1];
    assert.equal(createWrite.input.internal_port, 5000);
    assert.equal("subdomain" in createWrite.input, false);
    assert.equal("display_name" in createWrite.input, false);
    assert.deepEqual(unexpected, []);
    assert.deepEqual(errors, []);
    assert.equal(writes.filter(({ path }) => path.endsWith("/temporary-storage"))[0].input.expected_generation, 2);
    await context.close();
  } finally { await browser.close(); }
});
