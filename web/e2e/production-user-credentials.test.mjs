import assert from "node:assert/strict";
import { test } from "node:test";

const baseUrl = process.env.E2E_BASE_URL;
const adminToken = process.env.E2E_ADMIN_TOKEN;
const organizationId = process.env.E2E_ORGANIZATION_ID;
const targetUserId = process.env.E2E_TARGET_USER_ID;
const nodeWorkspaceId = process.env.E2E_NODE_WORKSPACE_ID;
const playwrightModule = process.env.E2E_PLAYWRIGHT_MODULE ?? "playwright-core";

function assertSafeE2eBaseUrl() {
  const url = new URL(baseUrl);
  const localHttp = process.env.E2E_LOCAL_BACKEND === "1"
    && url.protocol === "http:"
    && ["127.0.0.1", "::1", "localhost"].includes(url.hostname);
  assert.ok(url.protocol === "https:" || localHttp, "E2E_BASE_URL must be HTTPS or an explicitly enabled loopback HTTP backend");
}

async function api(path, options = {}) {
  const response = await fetch(new URL(path, baseUrl), {
    ...options,
    headers: { Authorization: `Bearer ${adminToken}`, "Content-Type": "application/json", ...options.headers },
  });
  assert.ok(response.ok, `${options.method ?? "GET"} ${path}: HTTP ${response.status}`);
  return response.status === 204 ? null : response.json();
}

async function findKey(name) {
  let cursor;
  do {
    const query = new URLSearchParams({ status: "all", limit: "100" });
    if (cursor) query.set("cursor", cursor);
    const page = await api(`/api/v1/admin/users/${targetUserId}/api-keys?${query}`);
    const match = page.items.find((item) => item.name === name);
    if (match) return match;
    cursor = page.next_cursor;
  } while (cursor);
  return null;
}

test("production administrator manages a specific user's credentials through the real API", { timeout: 120_000 }, async () => {
  assertSafeE2eBaseUrl();
  assert.ok(adminToken && organizationId && targetUserId, "Set E2E_ADMIN_TOKEN, E2E_ORGANIZATION_ID and E2E_TARGET_USER_ID");
  const principal = await api("/api/v1/me");
  assert.equal(principal.system_admin, true, "the browser principal must be a system administrator");
  for (const scope of ["manage_system", "manage_api_keys"]) assert.ok(principal.api_key_scopes.includes(scope), `missing ${scope}`);
  assert.equal(principal.allowed_template_ids, null, "administrator must not have template restrictions");
  assert.notEqual(principal.user_id, targetUserId, "target must be another specific user");
  const users = await api(`/api/v1/admin/users?organization_id=${organizationId}&limit=100`);
  const target = users.items.find((item) => item.id === targetUserId);
  assert.ok(target && !target.disabled, "target user must exist and be enabled");

  const { chromium } = await import(playwrightModule);
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_BIN || chromium.executablePath(), headless: true, args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  const keyName = `mwc-ui-e2e-${Date.now()}`;
  let createdId = null;
  try {
    const context = await browser.newContext({ permissions: ["clipboard-read", "clipboard-write"] });
    await context.addInitScript(({ token, organization }) => {
      sessionStorage.setItem("mwc.api-token", token);
      localStorage.setItem("mwc.locale", "en");
      localStorage.setItem("mwc.organization-id", organization);
      localStorage.setItem("mwc.view", "workspaces");
    }, { token: adminToken, organization: organizationId });
    const page = await context.newPage();
    await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
    await page.getByRole("button", { name: "Administration", exact: true }).click();
    await page.getByText("Users and roles", { exact: true }).waitFor();
    const row = page.getByRole("row", { name: new RegExp(target.display_name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")) });
    await row.getByRole("button", { name: "Credential configuration" }).click();
    const dialog = page.getByRole("dialog").filter({ hasText: target.display_name });
    await dialog.getByText(`Credential configuration · ${target.display_name}`, { exact: true }).waitFor();
    assert.equal(await dialog.getByRole("tab").count(), 0, "credentials must open directly, not through the old user editor tabs");
    await page.getByRole("button", { name: "Environment Variables & Files", exact: true }).waitFor();

    if (process.env.E2E_ALLOW_WRITES !== "1") return;
    await dialog.getByRole("button", { name: "Create API key" }).first().click();
    await dialog.getByRole("textbox", { name: "Key name" }).fill(keyName);
    await dialog.getByRole("button", { name: "Create API key" }).last().click();
    const keyRow = dialog.getByRole("row", { name: new RegExp(keyName) });
    await keyRow.getByRole("button", { name: /Copy|Copied/ }).waitFor();
    const listed = await findKey(keyName);
    createdId = listed?.id ?? null;
    assert.ok(listed?.id && listed.token, "real API list must return the created key and copyable token");
    await keyRow.getByRole("button", { name: /Copy|Copied/ }).click();
    assert.equal(await page.evaluate(() => navigator.clipboard.readText()), listed.token);
    await keyRow.getByRole("button", { name: "Revoke" }).click();
    await page.getByRole("textbox", { name: "Reason for revocation" }).fill("MWC UI E2E acceptance");
    await page.getByRole("button", { name: "Revoke", exact: true }).last().click();
    await keyRow.getByText("Revoked", { exact: true }).waitFor();
    assert.ok((await findKey(keyName))?.revoked_at, "real API must persist revocation");
  } finally {
    if (process.env.E2E_ALLOW_WRITES === "1") {
      const remaining = await findKey(keyName);
      if (remaining && !remaining.revoked_at) {
        await api(`/api/v1/admin/users/${targetUserId}/api-keys/${createdId ?? remaining.id}`, { method: "DELETE", body: JSON.stringify({ reason: "MWC UI E2E cleanup" }) });
      }
    }
    await browser.close();
  }
});

test("production labels distinguish environment files from user API credentials", { timeout: 90_000 }, async () => {
  assert.ok(baseUrl?.startsWith("https://") && adminToken && organizationId && targetUserId, "Set the production E2E URL, token, organization and user");
  const { chromium } = await import(playwrightModule);
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_BIN || chromium.executablePath(), headless: true, args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  try {
    for (const [locale, administration, files, users, credentials] of [
      ["zh-CN", "管理", "环境变量与文件", "用户与角色", "凭据配置"],
      ["ru", "Управление", "Переменные окружения и файлы", "Пользователи и роли", "Настройка ключей"],
    ]) {
      const context = await browser.newContext();
      await context.addInitScript(({ token, organization, language }) => {
        sessionStorage.setItem("mwc.api-token", token);
        localStorage.setItem("mwc.locale", language);
        localStorage.setItem("mwc.organization-id", organization);
        localStorage.setItem("mwc.view", "workspaces");
      }, { token: adminToken, organization: organizationId, language: locale });
      const page = await context.newPage();
      await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
      await page.getByRole("button", { name: files, exact: true }).waitFor();
      await page.getByRole("button", { name: administration, exact: true }).click();
      await page.getByText(users, { exact: true }).waitFor();
      await page.getByRole("button", { name: credentials, exact: true }).first().waitFor();
      await context.close();
    }
  } finally {
    await browser.close();
  }
});

test("production injection editor enables save only for changed fields", { timeout: 90_000 }, async () => {
  assert.ok(baseUrl?.startsWith("https://") && adminToken && organizationId, "Set the production E2E URL, token and organization");
  const { chromium } = await import(playwrightModule);
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_BIN || chromium.executablePath(), headless: true, args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  try {
    const context = await browser.newContext();
    await context.addInitScript(({ token, organization }) => {
      sessionStorage.setItem("mwc.api-token", token);
      localStorage.setItem("mwc.locale", "en");
      localStorage.setItem("mwc.organization-id", organization);
      localStorage.setItem("mwc.view", "workspaces");
    }, { token: adminToken, organization: organizationId });
    const page = await context.newPage();
    await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
    await page.getByRole("button", { name: "Environment Variables & Files", exact: true }).click();
    const editor = page.locator("form").filter({ hasText: "New credential" });
    const save = editor.getByRole("button", { name: "Create", exact: true });
    await save.waitFor();
    assert.equal(await save.isDisabled(), true);
    await save.hover({ force: true });
    await page.getByText("No modified content", { exact: true }).waitFor();
    const name = editor.getByRole("textbox", { name: "Name" });
    await name.fill("mwc-ui-e2e-unsaved");
    assert.equal(await save.isEnabled(), true);
    await editor.getByText("After saving, matching running workspaces update this file in place; stopped workspaces use the new content on their next start.").waitFor();
    await name.clear();
    assert.equal(await save.isDisabled(), true);
    await context.close();
  } finally {
    await browser.close();
  }
});

test("production workspace list shows the runtime Kubernetes node", { skip: !nodeWorkspaceId, timeout: 90_000 }, async () => {
  assert.ok(baseUrl?.startsWith("https://") && adminToken && organizationId, "Set the production E2E URL, token and organization");
  const runtimes = await api(`/api/v1/workspace-runtimes?organization_id=${organizationId}&workspace_ids=${nodeWorkspaceId}`);
  const runtime = runtimes.find((entry) => entry.workspace_id === nodeWorkspaceId)?.runtime;
  assert.ok(runtime?.node_name, "selected workspace must be scheduled on a node");
  const { chromium } = await import(playwrightModule);
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_BIN || chromium.executablePath(), headless: true, args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  try {
    const context = await browser.newContext();
    await context.addInitScript(({ token, organization }) => {
      sessionStorage.setItem("mwc.api-token", token);
      localStorage.setItem("mwc.locale", "en");
      localStorage.setItem("mwc.organization-id", organization);
      localStorage.setItem("mwc.view", "workspaces");
    }, { token: adminToken, organization: organizationId });
    const page = await context.newPage();
    await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
    await page.getByText(`Kubernetes node: ${runtime.node_name}`, { exact: true }).first().waitFor();
    await context.close();
  } finally {
    await browser.close();
  }
});

test("production Settings uses the shared Fluent form controls", { timeout: 90_000 }, async () => {
  assert.ok(baseUrl?.startsWith("https://") && adminToken && organizationId, "Set the production E2E URL, token and organization");
  const { chromium } = await import(playwrightModule);
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_BIN || chromium.executablePath(), headless: true, args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  try {
    const context = await browser.newContext();
    await context.addInitScript(({ token, organization }) => {
      sessionStorage.setItem("mwc.api-token", token);
      localStorage.setItem("mwc.locale", "en");
      localStorage.setItem("mwc.organization-id", organization);
      localStorage.setItem("mwc.view", "workspaces");
    }, { token: adminToken, organization: organizationId });
    const page = await context.newPage();
    await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await page.getByRole("textbox", { name: "Display name" }).waitFor();
    await page.getByRole("combobox", { name: "Current organization" }).waitFor();
    await page.getByRole("button", { name: "Save profile" }).waitFor();
    await context.close();
  } finally {
    await browser.close();
  }
});
