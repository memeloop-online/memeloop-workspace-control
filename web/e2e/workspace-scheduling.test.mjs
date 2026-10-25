import assert from "node:assert/strict";
import { mkdir, readFile } from "node:fs/promises";
import { test } from "node:test";
import { chromium } from "playwright-core";

const baseUrl = process.env.E2E_BASE_URL ?? "http://127.0.0.1:4173";
const organizationId = "scheduling-organization";
const resources = { cpu_millis: 2000, memory_mib: 4096, gpu_count: 0, disk_gib: 24 };
const names = ["Capacity blocked workspace", "Historical scheduling event workspace", "Malformed cause workspace", "Native event diagnostics workspace"];
const observedAt = "2026-10-18T12:14:00Z";
const nativeEvents = [
  { id: "native-container-eviction", category: "evicted", count: 1, observed_at: observedAt, last_observed_at: observedAt,
    details: { event_uid: "10000000-0000-4000-8000-000000000001", pod_uid: "20000000-0000-4000-8000-000000000001",
      reason: "Evicted", container: "workspace", scheduling_blockers: [],
      diagnostic: { kind: "container_local_ephemeral_limit", container: "workspace", limit_bytes: 2 * 1024 ** 3 } } },
  // Same category and observation time, but a different native event and cause.
  { id: "native-node-eviction", category: "evicted", count: 3, observed_at: observedAt, last_observed_at: observedAt,
    details: { event_uid: "10000000-0000-4000-8000-000000000002", pod_uid: "20000000-0000-4000-8000-000000000002",
      reason: "Evicted", container: null, scheduling_blockers: [],
      diagnostic: { kind: "node_resource_pressure", resource: "ephemeral_storage" } } },
  { id: "native-pod-limit", category: "ephemeral_storage", count: 1, observed_at: observedAt, last_observed_at: observedAt,
    details: { event_uid: "10000000-0000-4000-8000-000000000003", pod_uid: "20000000-0000-4000-8000-000000000003",
      reason: "Evicted", container: null, scheduling_blockers: [],
      diagnostic: { kind: "pod_local_ephemeral_limit", limit_bytes: 6 * 1024 ** 3 } } },
  { id: "native-memory-pressure", category: "memory_pressure", count: 1, observed_at: observedAt, last_observed_at: observedAt,
    details: { event_uid: "10000000-0000-4000-8000-000000000004", pod_uid: "20000000-0000-4000-8000-000000000004",
      reason: "Evicted", container: null, scheduling_blockers: [],
      diagnostic: { kind: "node_resource_pressure", resource: "memory" } } },
  { id: "native-pid-pressure", category: "pid_pressure", count: 1, observed_at: observedAt, last_observed_at: observedAt,
    details: { event_uid: "10000000-0000-4000-8000-000000000005", pod_uid: "20000000-0000-4000-8000-000000000005",
      reason: "Evicted", container: null, scheduling_blockers: [],
      diagnostic: { kind: "node_resource_pressure", resource: "pids" } } },
  { id: "historical-other", category: "other", count: 6, observed_at: observedAt, last_observed_at: null,
    details: { event_uid: null, pod_uid: null, reason: null, container: null, scheduling_blockers: [], diagnostic: null } },
];
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
    events: index === 3 ? nativeEvents : [{ id: `scheduling-event-${index}`, category: "scheduling", count: 1,
      observed_at: observedAt, last_observed_at: observedAt,
      details: { event_uid: index === 1 ? null : `scheduling-native-event-${index}`, pod_uid: null,
        reason: index === 1 ? null : "FailedScheduling", container: null, diagnostic: null,
        scheduling_blockers: index === 0 ? ["cpu_capacity", "storage_capacity", "cpu_capacity", "node_affinity"]
          : index === 2 ? ["__proto__", "unknown_cause", null, {}] : [] },
    }],
  },
}));
const summary = { total_count: workspaces.length, requested: resources, temporary_requested_gib: 32, state_counts: { starting: workspaces.length } };
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
    coverage: { total_workspaces: workspaces.length, eligible_workspaces: workspaces.length, template_label_coverage: "complete" },
  };
  if (path === "/api/v1/templates") return [{ id: "fixture-template", name: "Fixture template", enabled: true }];
  if (path === "/api/v1/node-pools") return [{ name: "default", display_name: "Fixture pool" }];
  if (path === "/api/v1/me/api-keys" || path.startsWith("/api/v1/injections/") || path === "/api/v1/webhooks") return [];
  return undefined;
}

test("workspace event log preserves native identity and distinguishes local limits from node pressure in each language", { timeout: 90_000 }, async () => {
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
      for (const [index, name] of names.slice(0, 3).entries()) {
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
      const nativeCard = page.getByRole("group", { name: names[3], exact: true });
      await nativeCard.getByRole("button", { name: messages.eventLog, exact: true }).click();
      const nativeLog = page.locator("#workspace-scheduling-3-event-log");
      const rows = nativeLog.getByRole("article");
      await rows.nth(nativeEvents.length - 1).waitFor();
      assert.equal(await rows.count(), nativeEvents.length);
      // Native UIDs remain in closed details, so they identify the exact article
      // without relying on translated summary copy or category/time row keys.
      const nativeRows = nativeEvents.slice(0, -1).map((event) => rows.filter({
        has: page.getByText(event.details.event_uid, { exact: true }),
      }));
      for (const row of nativeRows) assert.equal(await row.count(), 1);
      const [containerRow, nodeRow, podRow, memoryRow, pidRow] = nativeRows;
      assert.match(await containerRow.innerText(), /\bworkspace\b/);
      assert.match(await containerRow.innerText(), /\b2\.0\s*GiB\b/);
      const nativeReason = containerRow.locator("span > span").filter({ hasText: /^Evicted$/ });
      assert.equal(await nativeReason.count(), 1);
      await nativeReason.waitFor();
      assert.equal(await containerRow.getByText(messages.eventDiagnosticNodeEphemeralPressure, { exact: true }).count(), 0);
      await nodeRow.getByText(messages.eventDiagnosticNodeEphemeralPressure, { exact: true }).waitFor();
      assert.doesNotMatch(await nodeRow.innerText(), /\b2\.0\s*GiB\b/);
      assert.match(await podRow.innerText(), /\b6\.0\s*GiB\b/);
      assert.equal(await podRow.getByText(messages.eventDiagnosticNodeEphemeralPressure, { exact: true }).count(), 0);
      await memoryRow.getByText(messages.eventDiagnosticNodeMemoryPressure, { exact: true }).waitFor();
      await pidRow.getByText(messages.eventDiagnosticNodePidPressure, { exact: true }).waitFor();
      for (const [index, row] of nativeRows.entries()) {
        const event = nativeEvents[index];
        assert.equal(await row.getByText(event.details.event_uid, { exact: true }).isVisible(), false);
        await row.locator("details > summary").click();
        await row.getByText(event.details.event_uid, { exact: true }).waitFor();
        await row.getByText(event.details.pod_uid, { exact: true }).waitFor();
      }
      const historicalRow = rows.filter({ has: page.getByText(messages.eventCategoryOther, { exact: true }) });
      assert.equal(await historicalRow.count(), 1);
      assert.equal(await historicalRow.locator("details").count(), 0);
      assert.equal(await historicalRow.getByText("workspace", { exact: true }).count(), 0);
      assert.doesNotMatch(await historicalRow.innerText(), /Evicted|\b\d+\s*GiB\b|[0-9a-f]{8}-[0-9a-f-]{27,}/i);
      assert.deepEqual(unexpected, [], `${locale}: unexpected requests`);
      assert.deepEqual(errors, [], `${locale}: browser errors`);
      if (process.env.RUNNER_TEMP) {
        const screenshotDir = `${process.env.RUNNER_TEMP}/mwc-settings-screenshots`;
        await mkdir(screenshotDir, { recursive: true });
        await page.screenshot({ path: `${screenshotDir}/events-${locale}.png`, fullPage: true });
      }
      await context.close();
    }
  } finally { await browser.close(); }
});
