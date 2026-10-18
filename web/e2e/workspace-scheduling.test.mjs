import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { chromium } from "playwright-core";

const baseUrl = process.env.E2E_BASE_URL ?? "http://127.0.0.1:4173";
const organizationId = "scheduling-organization";
const resources = { cpu_millis: 2000, memory_mib: 4096, gpu_count: 0, disk_gib: 24 };
const names = ["Capacity blocked workspace", "Older API workspace", "Malformed cause workspace"];
const workspaces = names.map((name, index) => ({
  workspace: {
    id: `scheduling-${index}`, short_id: `scheduling-${index}`, name, state: "starting", resources,
    organization_id: organizationId, owner_id: "fixture-user", template_id: "fixture-template",
    node_pool: "default", image: "example.invalid/fixture:latest", access_mode: "internal",
    pod_requests: { cpu_millis: 2000, memory_mib: 4096 }, workspace_user: "fixture", workspace_home: "/home/fixture",
    buildkit: false, storage_policy: { temporary_storage_gib: 8 }, cluster_access: false,
    egress_policy: "internet_only", runtime_class_name: null,
    placement: { allowed_node_pools: ["default"], default_node_pool: "default" },
    generation: 1, created_at: 1_700_000_000, updated_at: 1_700_000_000,
  },
  namespace: "fixture", ssh_connection: null, ssh_host: null, ssh_port: null, ssh_command: null,
  ssh_config: null, web_shell_url: null, injection_sources: [], workspace_host_key: null, jump_host_key: null,
}));
const runtimes = workspaces.map(({ workspace }, index) => ({
  workspace_id: workspace.id,
  runtime: {
    allocated: resources, persistent_storage: { configured_bytes: 24 * 1024 ** 3, used_bytes: null, capacity_bytes: null, available_bytes: null, observed_at: null, used_percent: null, coverage: "unavailable", backing: "persistent_volume", pressure: "unknown" },
    temporary_storage: { configured_bytes: 8 * 1024 ** 3, used_bytes: null, capacity_bytes: null, available_bytes: null, observed_at: null, used_percent: null, coverage: "unavailable", backing: "ephemeral_volume", pressure: "unknown" },
    metrics_available: false, node_name: null, pods: [], metrics: [],
    events: [{ category: "scheduling", count: 1, observed_at: "2026-10-18T12:14:00Z", reason: "FailedScheduling",
      ...(index === 0 ? { scheduling_blockers: ["cpu_capacity", "storage_capacity", "cpu_capacity", "node_affinity"] }
        : index === 2 ? { scheduling_blockers: ["__proto__", "unknown_cause", null, {}] } : {}),
    }],
  },
}));
const summary = { total_count: 3, requested: resources, temporary_requested_gib: 24, state_counts: { starting: 3 } };
function fixture(path) {
  if (path === "/api/v1/me") return {
    user_id: "fixture-user", display_name: "Scheduling fixture", system_admin: false,
    memberships: [{ organization_id: organizationId, role: "member" }], api_key_scopes: ["read_workspace"],
    api_key_expires_at: 2_100_000_000, allowed_template_ids: null,
  };
  if (path === "/api/v1/me/profile") return { display_name: "Scheduling fixture", avatar_url: null };
  if (path === "/api/v1/organizations") return { items: [{ id: organizationId, name: "Scheduling fixture", created_at: 1_700_000_000 }], next_cursor: null };
  if (path === "/api/v1/workspaces") return { items: workspaces, next_cursor: null, summary };
  if (path === "/api/v1/workspace-runtimes") return runtimes;
  const runtime = /^\/api\/v1\/workspaces\/scheduling-(\d)\/runtime$/.exec(path);
  if (runtime) return runtimes[Number(runtime[1])].runtime;
  if (path === `/api/v1/organizations/${organizationId}/quota`) return null;
  if (path === `/api/v1/organizations/${organizationId}/usage-summary`) return {
    ...summary, actual: { cpu_millis: null, memory_mib: null, disk_bytes: null, temporary_bytes: null }, observed_at: null,
    availability: { cpu: "unknown", memory: "unknown", disk: "unknown", temporary: "unknown" },
    coverage: { total_workspaces: 3, eligible_workspaces: 3, template_label_coverage: "complete" },
  };
  if (path === "/api/v1/templates") return [{ id: "fixture-template", name: "Fixture template", enabled: true }];
  if (path === "/api/v1/node-pools") return [{ name: "default", display_name: "Fixture pool" }];
  if (path === "/api/v1/me/api-keys" || path.startsWith("/api/v1/injections/") || path === "/api/v1/webhooks") return [];
  return undefined;
}

test("workspace event log explains mixed scheduler causes and safely falls back in each language", { timeout: 90_000 }, async () => {
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_BIN || chromium.executablePath(), headless: true, args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  try {
    for (const locale of ["en", "zh-CN", "ru"]) {
      const messages = JSON.parse(await readFile(new URL(`../src/locales/${locale}.json`, import.meta.url), "utf8"));
      const context = await browser.newContext();
      const unexpected = [];
      const errors = [];
      await context.addInitScript(({ locale, organizationId }) => {
        sessionStorage.setItem("mwc.api-token", "scheduling-fixture-token");
        localStorage.setItem("mwc.locale", locale);
        localStorage.setItem("mwc.organization-id", organizationId);
        localStorage.setItem("mwc.view", "workspaces");
      }, { locale, organizationId });
      await context.route("**/*", async (route) => {
        const request = route.request();
        const url = new URL(request.url());
        if (url.origin !== new URL(baseUrl).origin) { unexpected.push("external request"); return route.abort(); }
        if (!url.pathname.startsWith("/api/")) return route.continue();
        const body = request.method() === "GET" ? fixture(url.pathname) : undefined;
        if (body === undefined) unexpected.push(`${request.method()} ${url.pathname}`);
        await route.fulfill({ status: body === undefined ? 501 : 200, contentType: "application/json", body: JSON.stringify(body ?? null) });
      });
      const page = await context.newPage();
      page.on("pageerror", (error) => errors.push(error.message));
      await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
      for (const [index, name] of names.entries()) {
        const card = page.getByRole("group", { name, exact: true });
        await card.getByRole("button", { name: messages.eventLog, exact: true }).click();
        const eventLog = page.locator(`#workspace-scheduling-${index}-event-log`);
        if (index === 0) {
          for (const key of ["eventSchedulingBlockerCpuCapacity", "eventSchedulingBlockerStorageCapacity", "eventSchedulingBlockerNodeAffinity"]) {
            await eventLog.getByText(messages[key], { exact: true }).waitFor();
            assert.equal(await eventLog.getByText(messages[key], { exact: true }).count(), 1);
          }
          assert.equal(await eventLog.getByText(messages.eventCategoryScheduling, { exact: true }).count(), 0);
        } else {
          await eventLog.getByText(messages.eventCategoryScheduling, { exact: true }).waitFor();
          assert.equal(await eventLog.getByText(messages.eventSchedulingBlockerCpuCapacity, { exact: true }).count(), 0);
        }
        assert.doesNotMatch(await eventLog.innerText(), /__proto__|unknown_cause|\[object Object\]/);
      }
      assert.deepEqual(unexpected, [], `${locale}: unexpected requests`);
      assert.deepEqual(errors, [], `${locale}: browser errors`);
      await context.close();
    }
  } finally { await browser.close(); }
});
