import assert from "node:assert/strict";
import { test as nodeTest } from "node:test";

const baseUrl = process.env.E2E_BASE_URL;
const organizationId = process.env.E2E_ORGANIZATION_ID;
const organizationAdminToken = process.env.E2E_ORGANIZATION_ADMIN_TOKEN;
const organizationAdminUserId = process.env.E2E_ORGANIZATION_ADMIN_USER_ID;
const playwrightModule = process.env.E2E_PLAYWRIGHT_MODULE ?? "playwright-core";
const test = baseUrl && organizationId && organizationAdminToken && organizationAdminUserId ? nodeTest : nodeTest.skip;

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
    headers: { Authorization: `Bearer ${organizationAdminToken}`, "Content-Type": "application/json", ...options.headers },
  });
  assert.ok(response.ok, `${options.method ?? "GET"} ${path}: HTTP ${response.status}`);
  return response.status === 204 ? null : response.json();
}

async function findOwnKey(name) {
  return (await api("/api/v1/me/api-keys")).find((item) => item.name === name) ?? null;
}

test("production organization administrator manages their own credentials through the real API", { timeout: 120_000 }, async () => {
  assertSafeE2eBaseUrl();
  const principal = await api("/api/v1/me");
  assert.equal(principal.system_admin, false, "the self-service principal must not be a system administrator");
  assert.equal(principal.user_id, organizationAdminUserId);
  assert.ok(principal.api_key_scopes.includes("manage_api_keys"), "organization administrator needs manage_api_keys");
  assert.ok(principal.memberships.some((membership) => membership.organization_id === organizationId && membership.role === "organization_admin"), "principal must be an organization administrator");

  const { chromium } = await import(playwrightModule);
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_BIN || chromium.executablePath(), headless: true, args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  const keyName = `mwc-ui-e2e-self-${Date.now()}`;
  let createdId = null;
  try {
    const context = await browser.newContext({ permissions: ["clipboard-read", "clipboard-write"] });
    await context.addInitScript(({ token, organization }) => {
      sessionStorage.setItem("mwc.api-token", token);
      localStorage.setItem("mwc.locale", "en");
      localStorage.setItem("mwc.organization-id", organization);
      localStorage.setItem("mwc.view", "workspaces");
    }, { token: organizationAdminToken, organization: organizationId });
    const page = await context.newPage();
    await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
    await page.getByRole("button", { name: "Administration", exact: true }).click();
    await page.getByText("Credential configuration", { exact: true }).waitFor();
    assert.equal(await page.getByText("Users and roles", { exact: true }).count(), 0, "organization administrators must not receive the system user directory");
    if (process.env.E2E_ALLOW_WRITES !== "1") return;

    await page.getByRole("button", { name: "Create API key" }).first().click();
    await page.getByRole("textbox", { name: "Key name" }).fill(keyName);
    await page.getByRole("button", { name: "Create API key" }).last().click();
    const keyRow = page.getByRole("row", { name: new RegExp(keyName) });
    await keyRow.getByRole("button", { name: /Copy|Copied/ }).waitFor();
    const listed = await findOwnKey(keyName);
    createdId = listed?.id ?? null;
    assert.ok(createdId, "the self-service API must persist the created key");
    await keyRow.getByRole("button", { name: /Copy|Copied/ }).click();
    assert.match(await page.evaluate(() => navigator.clipboard.readText()), /.+/, "the created key must be copyable before revocation");
    await keyRow.getByRole("button", { name: "Revoke" }).click();
    await page.getByRole("button", { name: "Revoke", exact: true }).last().click();
    await keyRow.waitFor({ state: "detached" });
    assert.equal(await findOwnKey(keyName), null, "the self-service API must remove the revoked key");
    await context.close();
  } finally {
    if (createdId && await findOwnKey(keyName)) {
      await api(`/api/v1/me/api-keys/${createdId}`, { method: "DELETE" });
    }
    await browser.close();
  }
});
