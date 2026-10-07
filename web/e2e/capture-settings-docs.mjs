import { mkdir } from "node:fs/promises";
import { join, resolve } from "node:path";
import { chromium } from "playwright-core";

const baseUrl = process.env.E2E_BASE_URL ?? process.env.baseURL ?? process.env.BASE_URL ?? "http://127.0.0.1:4173";
const screenshotDir = resolve(process.env.SCREENSHOT_DIR ?? "./settings-screenshots");
const origin = new URL(baseUrl).origin;
const organizationId = "demo-organization";
const demoKey = {
  id: "demo-key", name: "Demo automation key", prefix: "demo…",
  scopes: ["read_workspace", "manage_api_keys"], created_at: 1_700_000_000,
  last_used_at: null, expires_at: 2_100_000_000,
  allowed_template_ids: null, revoked_at: null,
};
const emptyPage = { items: [], next_cursor: null };
const requested = { cpu_millis: 0, memory_mib: 0, gpu_count: 0, disk_gib: 0 };
const unexpected = [];
const demoWorkspaces = [
  { id: "demo-web", short_id: "demo-web", name: "Demo Web Workspace", state: "ready", resources: { cpu_millis: 2000, memory_mib: 4096, gpu_count: 0, disk_gib: 24 } },
  { id: "demo-docs", short_id: "demo-docs", name: "Demo Docs Workspace", state: "stopped", resources: { cpu_millis: 1000, memory_mib: 2048, gpu_count: 0, disk_gib: 12 } },
].map(({ id, short_id, name, state, resources }) => ({
  workspace: {
    id, short_id, name, state, resources, organization_id: organizationId,
    owner_id: "demo-admin", template_id: "demo-template", node_pool: "default",
    image: "example.invalid/mwc-demo:latest", access_mode: "internal",
    pod_requests: { cpu_millis: resources.cpu_millis, memory_mib: resources.memory_mib },
    workspace_user: "demo", workspace_home: "/home/demo", buildkit: false,
    storage_policy: { temporary_storage_gib: 8 }, cluster_access: false,
    egress_policy: "internet_only", runtime_class_name: null,
    placement: { allowed_node_pools: ["default"], default_node_pool: "default" },
    generation: 1, created_at: 1_700_000_000, updated_at: 1_700_000_000,
  },
  namespace: "mwc-demo", ssh_connection: null, ssh_host: null, ssh_port: null,
  ssh_command: null, ssh_config: null, web_shell_url: null, injection_sources: [],
  workspace_host_key: null, jump_host_key: null,
}));
const demoInjection = {
  key: "demo-editor-mode", kind: "environment_variable", target: "EDITOR_MODE",
  scope: "user", scope_id: "demo-admin", sensitive: false, locked: false,
  version: 1, file_mode: null, owner: null, group: null, template_selector: null,
  labels: {}, updated_at: 1_700_000_000, value: { encoding: "utf8", value: "demo" },
};
const storage = (gib, usedGib, backing) => ({
  configured_bytes: gib * 1024 ** 3, used_bytes: usedGib * 1024 ** 3,
  capacity_bytes: gib * 1024 ** 3, available_bytes: (gib - usedGib) * 1024 ** 3,
  observed_at: 1_700_000_000, used_percent: usedGib / gib * 100,
  pressure: "normal", coverage: "exact", backing,
});
const demoRuntimes = demoWorkspaces.map(({ workspace }) => ({
  workspace_id: workspace.id,
  runtime: {
    allocated: workspace.resources,
    persistent_storage: storage(workspace.resources.disk_gib, workspace.id === "demo-web" ? 3 : 2, "persistent_volume"),
    temporary_storage: storage(8, workspace.id === "demo-web" ? 1 : 0, "node_local"),
    metrics_available: workspace.state === "ready", node_name: workspace.state === "ready" ? "demo-node" : null,
    pods: [], metrics: [], events: [],
  },
}));

function fixture(path) {
  if (path === "/api/v1/me") return {
    user_id: "demo-admin", display_name: "Alex Demo", system_admin: true,
    memberships: [{ organization_id: organizationId, role: "organization_admin" }],
    api_key_scopes: ["manage_system", "manage_organization", "manage_members", "manage_api_keys", "read_workspace"],
    api_key_expires_at: 2_100_000_000, allowed_template_ids: null,
  };
  if (path === "/api/v1/me/profile") return { display_name: "Alex Demo", avatar_url: null };
  if (path === "/api/v1/me/api-keys") return [demoKey];
  if (path === "/api/v1/organizations") return { items: [{ id: organizationId, name: "MWC Demo", created_at: 1_700_000_000 }], next_cursor: null };
  if (path === "/api/v1/workspaces") return {
    ...emptyPage, items: demoWorkspaces,
    summary: { total_count: 2, requested: { cpu_millis: 3000, memory_mib: 6144, gpu_count: 0, disk_gib: 36 }, temporary_requested_gib: 16, state_counts: { ready: 1, stopped: 1 } },
  };
  if (path === "/api/v1/workspace-runtimes") return demoRuntimes;
  if (path === `/api/v1/workspaces/demo-web/runtime`) return demoRuntimes[0].runtime;
  if (path === `/api/v1/workspaces/demo-docs/runtime`) return demoRuntimes[1].runtime;
  if (path === "/api/v1/injections/user/demo-admin") return [demoInjection];
  if (path === `/api/v1/injections/organization/${organizationId}` || path === "/api/v1/injections/workspace/demo-web" || path === "/api/v1/injections/workspace/demo-docs") return [];
  if (path === `/api/v1/organizations/${organizationId}/quota`) return null;
  if (path === `/api/v1/organizations/${organizationId}/usage-summary`) return {
    total_count: 2, state_counts: { ready: 1, stopped: 1 },
    requested: { cpu_millis: 3000, memory_mib: 6144, gpu_count: 0, disk_gib: 36 }, temporary_requested_gib: 16,
    actual: { cpu_millis: 320, memory_mib: 768, disk_bytes: 5 * 1024 ** 3, temporary_bytes: 1024 ** 3 },
    observed_at: 1_700_000_000,
    availability: { cpu: "available", memory: "available", disk: "available", temporary: "available" },
    coverage: { total_workspaces: 2, eligible_workspaces: 2, template_label_coverage: "complete" },
  };
  if (path === "/api/v1/admin/users") return { ...emptyPage, items: [{
    id: "demo-admin", display_name: "Alex Demo", system_admin: true,
    disabled: false, created_at: 1_700_000_000, membership_role: "organization_admin",
  }] };
  if (path === "/api/v1/templates") return [{
    id: "demo-template", organization_id: organizationId, name: "Demo Development",
    image: "example.invalid/mwc-demo:latest", access_mode: "internal",
    resources: demoWorkspaces[0].workspace.resources,
    pod_requests: { cpu_millis: 2000, memory_mib: 4096 }, workspace_user: "demo",
    workspace_home: "/home/demo", buildkit: false, storage_policy: { temporary_storage_gib: 8 },
    cluster_access: false, egress_policy: "internet_only", runtime_class_name: null,
    placement: { allowed_node_pools: ["default"], default_node_pool: "default" }, yaml: "", enabled: true,
  }];
  if (path === "/api/v1/webhooks" || path === "/api/v1/admin/images") return [];
  if (path === "/api/v1/node-pools") return [{ name: "default", display_name: "Demo pool" }];
  if (path === "/api/v1/admin/scaling") return { database_mode: "sqlite", configured_replicas: 1, schema_version: 1, jobs: { pending: 0, running: 0, completed: 0, failed: 0 } };
  if (path === "/api/v1/audit") return { items: [], next_offset: null };
  return undefined;
}

async function labelDemo(page) {
  await page.evaluate(() => {
    const badge = document.createElement("div");
    badge.textContent = "MWC DEMO · SANITIZED DATA · CI BROWSER CAPTURE";
    Object.assign(badge.style, {
      position: "fixed", right: "16px", bottom: "16px", zIndex: "2147483647",
      padding: "8px 12px", borderRadius: "6px", background: "#16324f",
      color: "#fff", font: "600 12px system-ui, sans-serif", letterSpacing: ".04em",
      boxShadow: "0 2px 12px #0006", pointerEvents: "none",
    });
    document.body.append(badge);
  });
}

await mkdir(screenshotDir, { recursive: true });
const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM_BIN || chromium.executablePath(),
  headless: true, args: ["--no-sandbox", "--disable-dev-shm-usage"],
});

try {
  const context = await browser.newContext({ viewport: { width: 1440, height: 960 }, deviceScaleFactor: 1 });
  await context.addInitScript(() => {
    sessionStorage.setItem("mwc.api-token", "sanitized-demo-auth-placeholder");
    localStorage.setItem("mwc.locale", "en");
    localStorage.setItem("mwc.organization-id", "demo-organization");
    localStorage.setItem("mwc.view", "workspaces");
  });
  await context.route("**/*", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    if (url.origin !== origin) {
      unexpected.push(`external request: ${url.origin}`);
      return route.abort();
    }
    if (!url.pathname.startsWith("/api/")) return route.continue();
    const response = request.method() === "GET" ? fixture(url.pathname) : undefined;
    if (request.method() !== "GET" || (response === undefined && url.pathname !== `/api/v1/organizations/${organizationId}/quota`)) {
      unexpected.push(`${request.method()} ${url.pathname}`);
      return route.fulfill({ status: 501, contentType: "application/json", body: "{}" });
    }
    await route.fulfill({ contentType: "application/json", body: JSON.stringify(response ?? null) });
  });

  const page = await context.newPage();
  await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByTestId("settings-section-profile").waitFor();
  await labelDemo(page);
  await page.screenshot({ path: join(screenshotDir, "settings-desktop.png"), fullPage: true, animations: "disabled" });

  await page.getByTestId("settings-search").fill("appearance");
  await page.getByTestId("settings-section-appearance").waitFor();
  await page.screenshot({ path: join(screenshotDir, "settings-search.png"), fullPage: true, animations: "disabled" });

  await page.getByTestId("settings-search").fill("");
  await page.getByRole("button", { name: "Personal", exact: true }).click();
  await page.getByTestId("settings-section-api-keys").waitFor();
  await page.getByText("Demo automation key", { exact: true }).waitFor();
  await page.screenshot({ path: join(screenshotDir, "user-api-keys.png"), fullPage: true, animations: "disabled" });

  if (unexpected.length) throw new Error(`Unexpected demo requests: ${unexpected.join(", ")}`);
  await context.close();
} finally {
  await browser.close();
}
