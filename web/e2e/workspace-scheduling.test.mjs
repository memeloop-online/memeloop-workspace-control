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

test("workspace creation always shows the run location and only submits usable locations", { timeout: 120_000 }, async () => {
  const messages = JSON.parse(await readFile(new URL("../src/locales/en.json", import.meta.url), "utf8"));
  const creationResources = { ...resources, disk_gib: 20 };
  const pools = [
    { name: "placement-westlake", display_name: "West Lake" },
    { name: "placement-haixia", display_name: "Strait" },
    { name: "placement-unrelated", display_name: "Unrelated location" },
  ];
  const templates = [
    { id: "rust-template", name: "Rust Dev", temporaryStorage: 10 },
    { id: "other-template", name: "Other development", temporaryStorage: 4 },
    { id: "wide-template", name: "Wide placement", temporaryStorage: 6, allowed: [pools[0].name, pools[1].name, pools[2].name] },
  ].map(({ id, name, temporaryStorage, allowed }) => ({
    id, name, organization_id: organizationId, enabled: true, yaml: "",
    image: "example.invalid/rust-dev:1", access_mode: "internal", resources: creationResources,
    pod_requests: { cpu_millis: 500, memory_mib: 512 }, cpu_overcommit_ratio: 10,
    workspace_user: "user", workspace_home: "/home/user", buildkit: true,
    storage_policy: { temporary_storage_gib: temporaryStorage }, cluster_access: false,
    egress_policy: "internet_only", runtime_class_name: null, desktop: null,
    placement: { allowed_node_pools: allowed ?? [pools[0].name, pools[1].name], default_node_pool: pools[0].name },
  }));
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_BIN || chromium.executablePath(), headless: true, args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  try {
    const openCreationPage = async ({ availablePools, nodePoolFailures = 0 }) => {
      const context = await browser.newContext();
      const unexpected = [];
      const errors = [];
      const submissions = [];
      let nodePoolRequests = 0;
      await context.addInitScript(({ organizationId }) => {
        sessionStorage.setItem("mwc.api-token", "scheduling-fixture-token");
        localStorage.setItem("mwc.locale", "en");
        localStorage.setItem("mwc.organization-id", organizationId);
        localStorage.setItem("mwc.view", "workspaces");
      }, { organizationId });
      await context.route("**/*", async (route) => {
        const request = route.request();
        const url = new URL(request.url());
        if (url.origin !== new URL(baseUrl).origin) { unexpected.push("external request"); return route.abort(); }
        if (!url.pathname.startsWith("/api/")) return route.continue();
        if (request.method() === "POST" && url.pathname === "/api/v1/workspaces") {
          submissions.push(request.postDataJSON());
          return route.fulfill({ status: 201, contentType: "application/json", body: JSON.stringify(workspaces[0]) });
        }
        let response = request.method() === "GET" ? fixture(url.pathname) : undefined;
        if (request.method() === "GET") {
          if (url.pathname === "/api/v1/me") response = { ...response, api_key_scopes: ["read_workspace", "create_workspace"] };
          if (url.pathname === "/api/v1/templates") response = templates;
          if (url.pathname === "/api/v1/node-pools") {
            nodePoolRequests += 1;
            if (nodePoolRequests <= nodePoolFailures) {
              return route.fulfill({ status: 500, contentType: "application/json", body: "{}" });
            }
            response = availablePools;
          }
        }
        if (response === undefined) unexpected.push(`${request.method()} ${url.pathname}`);
        await route.fulfill({ status: response === undefined ? 501 : 200, contentType: "application/json", body: JSON.stringify(response ?? null) });
      });
      const page = await context.newPage();
      page.on("pageerror", (error) => errors.push(error.message));
      await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
      await page.getByRole("button", { name: messages.newWorkspace, exact: true }).click();
      const chooseTemplate = async (name) => {
        await page.getByRole("combobox", { name: messages.template }).click();
        await page.getByRole("option", { name, exact: true }).click();
      };
      return { context, page, unexpected, errors, submissions, chooseTemplate };
    };

    // Multiple allowed locations: a real choice with an explicit template default.
    {
      const { context, page, unexpected, errors, submissions, chooseTemplate } = await openCreationPage({ availablePools: pools });
      await chooseTemplate(templates[0].name);
      const temporaryStorage = page.getByRole("spinbutton", { name: messages.workspaceTemporaryStorageGiB, exact: true });
      const homeStorage = page.getByRole("spinbutton", { name: messages.diskSizeGib, exact: true });
      assert.equal(await temporaryStorage.inputValue(), "10");
      assert.equal(await homeStorage.inputValue(), "20");
      assert.equal(await page.getByRole("spinbutton", { name: messages.cpuLimitMillis, exact: true }).inputValue(), String(creationResources.cpu_millis));
      assert.equal(await page.getByRole("spinbutton", { name: messages.memoryLimitMib, exact: true }).inputValue(), String(creationResources.memory_mib));
      assert.equal(await page.getByRole("spinbutton", { name: messages.gpuCount, exact: true }).inputValue(), "0");
      await page.getByText(messages.workspaceTemporaryStorageHelp, { exact: true }).waitFor();
      const location = page.getByRole("combobox", { name: messages.workspaceRunLocation, exact: true });
      await location.waitFor();
      assert.equal(await location.innerText(), messages.workspaceRunLocationTemplateDefault);
      await location.click();
      assert.equal(await page.getByRole("option", { name: pools[2].display_name, exact: true }).count(), 0);
      await page.getByRole("option", { name: pools[1].display_name, exact: true }).click();
      await temporaryStorage.fill("12");
      await homeStorage.fill("27");
      await chooseTemplate(templates[1].name);
      assert.equal(await temporaryStorage.inputValue(), "4");
      assert.equal(await homeStorage.inputValue(), "20");
      assert.equal(await location.innerText(), messages.workspaceRunLocationTemplateDefault, "switching the template resets the location choice");
      await chooseTemplate(templates[0].name);
      assert.equal(await temporaryStorage.inputValue(), "10");
      assert.equal(await homeStorage.inputValue(), "20");
      await page.getByRole("textbox", { name: messages.name, exact: true }).fill("Payment development");
      await temporaryStorage.fill("12");
      const firstPost = page.waitForRequest((request) => request.method() === "POST" && new URL(request.url()).pathname === "/api/v1/workspaces");
      await page.getByRole("button", { name: messages.submitCreate, exact: true }).click();
      const defaultPayload = (await firstPost).postDataJSON();
      assert.equal(defaultPayload.temporary_storage_gib, 12);
      assert.equal(defaultPayload.node_pool, null, "the template default is sent as null, not a guessed pool");
      assert.equal(defaultPayload.resources, null, "temporary capacity must not change template CPU, memory, GPU, or Home");
      assert.equal(defaultPayload.template_id, templates[0].id);
      await temporaryStorage.waitFor({ state: "hidden" });
      await page.getByRole("button", { name: messages.newWorkspace, exact: true }).click();
      await chooseTemplate(templates[0].name);
      await page.getByRole("textbox", { name: messages.name, exact: true }).fill("Payment development in Strait");
      await location.click();
      await page.getByRole("option", { name: pools[1].display_name, exact: true }).click();
      assert.equal(await location.innerText(), pools[1].display_name);
      const secondPost = page.waitForRequest((request) => request.method() === "POST" && new URL(request.url()).pathname === "/api/v1/workspaces");
      await page.getByRole("button", { name: messages.submitCreate, exact: true }).click();
      const selectedPayload = (await secondPost).postDataJSON();
      assert.equal(selectedPayload.node_pool, pools[1].name);
      assert.equal(selectedPayload.resources, null);
      assert.equal(Object.hasOwn(selectedPayload, "temporary_storage_gib"), false, "unchanged capacity inherits the template");
      await temporaryStorage.waitFor({ state: "hidden" });
      assert.equal(submissions.length, 2);
      assert.deepEqual(unexpected, [], "unexpected creation requests");
      assert.deepEqual(errors, [], "creation browser errors");
      await context.close();
    }

    // One allowed location: visible fixed human name, no hidden selector, and the
    // unrelated available pool must not turn it into a choice.
    {
      const { context, page, unexpected, errors, submissions, chooseTemplate } = await openCreationPage({ availablePools: [pools[0], pools[2]] });
      await chooseTemplate(templates[0].name);
      assert.equal(await page.getByRole("combobox", { name: messages.workspaceRunLocation, exact: true }).count(), 0);
      const fixed = page.getByRole("textbox", { name: messages.workspaceRunLocation, exact: true });
      await fixed.waitFor();
      assert.equal(await fixed.inputValue(), pools[0].display_name);
      await page.getByText(messages.workspaceRunLocationFixed, { exact: true }).waitFor();
      await page.getByRole("textbox", { name: messages.name, exact: true }).fill("Single location workspace");
      const post = page.waitForRequest((request) => request.method() === "POST" && new URL(request.url()).pathname === "/api/v1/workspaces");
      await page.getByRole("button", { name: messages.submitCreate, exact: true }).click();
      const payload = (await post).postDataJSON();
      assert.equal(payload.node_pool, null, "the single allowed location stays the template default");
      assert.equal(submissions.length, 1);
      assert.deepEqual(unexpected, [], "single location: unexpected requests");
      assert.deepEqual(errors, [], "single location: browser errors");
      await context.close();
    }

    // Disabled template default with a single enabled alternate: the fixed
    // location is submitted explicitly instead of pointing null at the default.
    {
      const { context, page, unexpected, errors, submissions, chooseTemplate } = await openCreationPage({ availablePools: [pools[1]] });
      await chooseTemplate(templates[0].name);
      const fixed = page.getByRole("textbox", { name: messages.workspaceRunLocation, exact: true });
      await fixed.waitFor();
      assert.equal(await fixed.inputValue(), pools[1].display_name);
      await page.getByText(messages.workspaceRunLocationFixed, { exact: true }).waitFor();
      await page.getByRole("textbox", { name: messages.name, exact: true }).fill("Alternate location workspace");
      const post = page.waitForRequest((request) => request.method() === "POST" && new URL(request.url()).pathname === "/api/v1/workspaces");
      await page.getByRole("button", { name: messages.submitCreate, exact: true }).click();
      const payload = (await post).postDataJSON();
      assert.equal(payload.node_pool, pools[1].name, "the single remaining location is sent explicitly");
      assert.equal(submissions.length, 1);
      assert.deepEqual(unexpected, [], "alternate location: unexpected requests");
      assert.deepEqual(errors, [], "alternate location: browser errors");
      await context.close();
    }

    // Disabled template default with several alternates: the default option is
    // never offered and an explicit choice is required before submitting.
    {
      const { context, page, unexpected, errors, submissions, chooseTemplate } = await openCreationPage({ availablePools: [pools[1], pools[2]] });
      await chooseTemplate(templates[2].name);
      const location = page.getByRole("combobox", { name: messages.workspaceRunLocation, exact: true });
      await location.waitFor();
      assert.equal((await location.innerText()).trim(), messages.workspaceRunLocationChoose, "an unusable default leaves a choose-location placeholder");
      await location.click();
      assert.equal(await page.getByRole("option", { name: messages.workspaceRunLocationTemplateDefault, exact: true }).count(), 0, "an unusable default is never offered");
      await page.getByRole("option", { name: pools[2].display_name, exact: true }).waitFor();
      await page.keyboard.press("Escape");
      await page.getByRole("textbox", { name: messages.name, exact: true }).fill("Needs an explicit location");
      const submit = page.getByRole("button", { name: messages.submitCreate, exact: true });
      assert.equal(await submit.isDisabled(), true, "creation requires an explicit location choice");
      await page.evaluate(() => document.querySelector("form")?.requestSubmit());
      await page.getByText(messages.workspaceRunLocationInvalid, { exact: true }).waitFor();
      assert.equal(submissions.length, 0, "an unset choice must not reach the API");
      await location.click();
      await page.getByRole("option", { name: pools[1].display_name, exact: true }).click();
      assert.equal(await submit.isDisabled(), false);
      const post = page.waitForRequest((request) => request.method() === "POST" && new URL(request.url()).pathname === "/api/v1/workspaces");
      await submit.click();
      const payload = (await post).postDataJSON();
      assert.equal(payload.node_pool, pools[1].name);
      assert.equal(submissions.length, 1);
      assert.deepEqual(unexpected, [], "explicit choice: unexpected requests");
      assert.deepEqual(errors, [], "explicit choice: browser errors");
      await context.close();
    }

    // No allowed location: an explicit message and no way to submit, even when
    // the form submit event is forced around the disabled button.
    {
      const { context, page, unexpected, errors, submissions, chooseTemplate } = await openCreationPage({ availablePools: [] });
      await chooseTemplate(templates[0].name);
      await page.getByText(messages.workspaceRunLocationNone, { exact: true }).waitFor();
      assert.equal(await page.getByRole("combobox", { name: messages.workspaceRunLocation, exact: true }).count(), 0);
      await page.getByRole("textbox", { name: messages.name, exact: true }).fill("Nowhere to run");
      const submit = page.getByRole("button", { name: messages.submitCreate, exact: true });
      assert.equal(await submit.isDisabled(), true);
      await page.evaluate(() => document.querySelector("form")?.requestSubmit());
      await page.waitForTimeout(300);
      assert.equal(submissions.length, 0, "a template without an available location must not be creatable");
      assert.deepEqual(unexpected, [], "no location: unexpected requests");
      assert.deepEqual(errors, [], "no location: browser errors");
      await context.close();
    }

    // Failed public list: explicit error with a retry that recovers the selector.
    {
      const { context, page, unexpected, errors, submissions, chooseTemplate } = await openCreationPage({ availablePools: pools, nodePoolFailures: 1 });
      await chooseTemplate(templates[0].name);
      await page.getByText(messages.runLocationsLoadFailed, { exact: true }).waitFor();
      const submit = page.getByRole("button", { name: messages.submitCreate, exact: true });
      assert.equal(await submit.isDisabled(), true, "creation stays blocked while locations failed to load");
      await page.getByRole("button", { name: messages.retry, exact: true }).click();
      const location = page.getByRole("combobox", { name: messages.workspaceRunLocation, exact: true });
      await location.waitFor();
      assert.equal(await location.innerText(), messages.workspaceRunLocationTemplateDefault);
      assert.equal(await submit.isDisabled(), false);
      await page.getByRole("textbox", { name: messages.name, exact: true }).fill("Recovered location load");
      const post = page.waitForRequest((request) => request.method() === "POST" && new URL(request.url()).pathname === "/api/v1/workspaces");
      await submit.click();
      const payload = (await post).postDataJSON();
      assert.equal(payload.node_pool, null);
      assert.equal(submissions.length, 1);
      assert.deepEqual(unexpected, [], "retry: unexpected requests");
      assert.deepEqual(errors, [], "retry: browser errors");
      await context.close();
    }
  } finally { await browser.close(); }
});

test("workspace sorting defaults to newest, resets paging on change, and ignores stale sort responses", { timeout: 90_000 }, async () => {
  const sortableNames = ["Alpha sorting workspace", "Beta sorting workspace", "Gamma sorting workspace"];
  const sortable = sortableNames.map((name, index) => ({
    workspace: {
      id: `sortable-${index}`, short_id: `sortable-${index}`, name, state: "starting", resources,
      organization_id: organizationId, owner_id: "fixture-user", template_id: "fixture-template",
      node_pool: "default", image: "example.invalid/fixture:latest", access_mode: "internal",
      pod_requests: { cpu_millis: 2000, memory_mib: 4096 }, workspace_user: "fixture", workspace_home: "/home/fixture",
      buildkit: false, storage_policy: { temporary_storage_gib: 8 }, cluster_access: false,
      egress_policy: "internet_only", runtime_class_name: null,
      placement: { allowed_node_pools: ["default"], default_node_pool: "default" },
      generation: 1, created_at: 1_700_000_000 + index, updated_at: 1_700_000_000 + index,
    },
    namespace: "fixture", ssh_connection: null, ssh_host: null, ssh_port: null, ssh_command: null,
    ssh_config: null, web_shell_url: null, injection_sources: [], workspace_host_key: null, jump_host_key: null,
  }));
  const [alpha, beta, gamma] = sortable;
  const sortSummary = { total_count: 3, requested: resources, temporary_requested_gib: 24, state_counts: { starting: 3 } };
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_BIN || chromium.executablePath(), headless: true, args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  try {
    for (const locale of ["en", "zh-CN", "ru"]) {
      const messages = JSON.parse(await readFile(new URL(`../src/locales/${locale}.json`, import.meta.url), "utf8"));
      const context = await browser.newContext();
      const unexpected = [];
      const errors = [];
      const listRequests = [];
      let oldestRequested;
      const oldestRequestSeen = new Promise((resolve) => { oldestRequested = resolve; });
      let releaseOldest;
      const oldestHeld = new Promise((resolve) => { releaseOldest = resolve; });
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
        if (request.method() === "GET" && url.pathname === "/api/v1/workspaces") {
          const sort = url.searchParams.get("sort");
          const cursor = url.searchParams.get("cursor");
          listRequests.push({ sort, cursor });
          if (sort === "oldest") {
            // Hold the old-sort response until the newest scope has settled, so a
            // late response can only corrupt the list if staleness checks fail.
            oldestRequested();
            await oldestHeld;
            return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ items: [alpha, beta], next_cursor: null, summary: sortSummary }) });
          }
          const pageItems = cursor === "newest-page-2" ? [alpha] : [gamma, beta];
          const nextCursor = cursor === "newest-page-2" ? null : "newest-page-2";
          return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ items: pageItems, next_cursor: nextCursor, summary: sortSummary }) });
        }
        if (request.method() === "GET" && url.pathname === "/api/v1/workspace-runtimes") {
          return route.fulfill({ status: 200, contentType: "application/json", body: "[]" });
        }
        const body = request.method() === "GET" ? fixture(url.pathname) : undefined;
        if (body === undefined) unexpected.push(`${request.method()} ${url.pathname}`);
        await route.fulfill({ status: body === undefined ? 501 : 200, contentType: "application/json", body: JSON.stringify(body ?? null) });
      });
      const page = await context.newPage();
      page.on("pageerror", (error) => errors.push(error.message));
      await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
      const sortControl = page.getByRole("combobox", { name: messages.sortWorkspaces, exact: true });
      await sortControl.waitFor();
      assert.equal((await sortControl.innerText()).trim(), messages.sortNewestFirst, `${locale}: default sort shows newest first`);
      await sortControl.click();
      await page.getByRole("option", { name: messages.sortNewestFirst, exact: true }).waitFor();
      await page.getByRole("option", { name: messages.sortOldestFirst, exact: true }).waitFor();
      await page.keyboard.press("Escape");
      await page.getByRole("group", { name: sortableNames[2], exact: true }).waitFor();
      assert.deepEqual(listRequests[0], { sort: "newest", cursor: null }, `${locale}: the default listing requests newest first without a cursor`);
      assert.deepEqual(unexpected, [], `${locale}: unexpected requests`);
      assert.deepEqual(errors, [], `${locale}: browser errors`);
      if (locale !== "en") { await context.close(); continue; }
      const headings = () => page.getByRole("heading", { level: 2 }).allInnerTexts();
      assert.deepEqual((await headings()).map((text) => text.trim()), [sortableNames[2], sortableNames[1]], "newest first: Gamma before Beta");
      await page.getByRole("button", { name: messages.nextPage, exact: true }).click();
      await page.getByRole("group", { name: sortableNames[0], exact: true }).waitFor();
      await page.getByText(`${messages.workspacePageStatus} 2`, { exact: true }).waitFor();
      assert.deepEqual(listRequests[1], { sort: "newest", cursor: "newest-page-2" }, "page two continues the newest keyset");
      await sortControl.click();
      await page.getByRole("option", { name: messages.sortOldestFirst, exact: true }).click();
      await oldestRequestSeen;
      assert.deepEqual(listRequests[2], { sort: "oldest", cursor: null }, "switching the sort restarts the keyset without the old cursor");
      await page.getByText(`${messages.workspacePageStatus} 1`, { exact: true }).waitFor();
      await sortControl.click();
      await page.getByRole("option", { name: messages.sortNewestFirst, exact: true }).click();
      await page.getByRole("group", { name: sortableNames[2], exact: true }).waitFor();
      assert.deepEqual(listRequests[3], { sort: "newest", cursor: null }, "switching back refetches the newest first page");
      const staleResponse = page.waitForResponse((response) => new URL(response.url()).pathname === "/api/v1/workspaces" && new URL(response.url()).searchParams.get("sort") === "oldest");
      releaseOldest();
      await staleResponse;
      // Let the page event loop process the late response before asserting it was discarded.
      await page.evaluate(() => new Promise((resolve) => setTimeout(resolve, 100)));
      assert.deepEqual((await headings()).map((text) => text.trim()), [sortableNames[2], sortableNames[1]], "a stale old-sort response must not overwrite the newest list");
      await page.getByText(`${messages.workspacePageStatus} 1`, { exact: true }).waitFor();
      assert.deepEqual(errors, [], "browser errors after stale response");
      await context.close();
    }
  } finally { await browser.close(); }
});
