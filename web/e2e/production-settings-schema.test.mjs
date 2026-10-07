import assert from "node:assert/strict";
import { test } from "node:test";

const baseUrl = process.env.E2E_BASE_URL;
const organizationId = process.env.E2E_ORGANIZATION_ID;
const playwrightModule = process.env.E2E_PLAYWRIGHT_MODULE ?? "playwright-core";

test("real Settings schema searches only sections visible to each role", { timeout: 150_000 }, async () => {
  const url = new URL(baseUrl);
  const localHttp = process.env.E2E_LOCAL_BACKEND === "1" && url.protocol === "http:" && ["127.0.0.1", "::1", "localhost"].includes(url.hostname);
  assert.ok(url.protocol === "https:" || localHttp, "E2E_BASE_URL must be HTTPS or an explicitly enabled loopback backend");
  assert.ok(organizationId && process.env.E2E_ADMIN_TOKEN && process.env.E2E_ORGANIZATION_ADMIN_TOKEN && process.env.E2E_TARGET_TOKEN, "Set the organization and all three role tokens");

  const { chromium } = await import(playwrightModule);
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_BIN || chromium.executablePath(), headless: true, args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  try {
    for (const [role, token] of [
      ["system", process.env.E2E_ADMIN_TOKEN],
      ["organization", process.env.E2E_ORGANIZATION_ADMIN_TOKEN],
      ["member", process.env.E2E_TARGET_TOKEN],
    ]) {
      const response = await fetch(new URL("/api/v1/me", baseUrl), { headers: { Authorization: `Bearer ${token}` } });
      assert.equal(response.status, 200, `${role} fixture must authenticate`);
      const principal = await response.json();
      assert.equal(principal.system_admin, role === "system", `${role} fixture has the wrong system role`);
      if (role !== "system") assert.equal(principal.memberships.find((membership) => membership.organization_id === organizationId)?.role, role === "organization" ? "organization_admin" : "member", `${role} fixture has the wrong organization role`);
      assert.equal(principal.api_key_scopes.includes("manage_api_keys"), role !== "member", `${role} fixture has the wrong key-management scope`);
      const context = await browser.newContext();
      await context.addInitScript(({ credential, organization }) => {
        sessionStorage.setItem("mwc.api-token", credential);
        localStorage.setItem("mwc.locale", "en");
        localStorage.setItem("mwc.organization-id", organization);
        localStorage.setItem("mwc.view", "workspaces");
      }, { credential: token, organization: organizationId });
      const page = await context.newPage();
      const forbiddenRequests = [];
      if (role === "member") page.on("request", (request) => {
        const pathname = new URL(request.url()).pathname;
        if (pathname.startsWith("/api/v1/admin/") || pathname.startsWith("/api/v1/me/api-keys")) forbiddenRequests.push(`${request.method()} ${pathname}`);
      });
      await page.goto(`${baseUrl}/#administration`, { waitUntil: "domcontentloaded" });
      await page.getByRole("button", { name: "Settings", exact: true }).waitFor();
      assert.equal(await page.getByRole("button", { name: "Administration", exact: true }).count(), 0, "the old navigation entry must be removed");

      const search = page.getByTestId("settings-search");
      await search.fill("display_name");
      await page.getByTestId("settings-section-profile").waitFor();
      await search.fill("no-such-settings-schema-entry");
      await page.getByText("No matching settings.", { exact: true }).waitFor();
      assert.equal(await page.getByTestId("settings-section-profile").count(), 0, "unmatched search must show no matching section");

      await search.fill("users");
      if (role === "member") {
        assert.equal(await page.getByTestId("settings-section-users").count(), 0, "search must not discover hidden administrator sections");
      } else {
        const usersSection = page.getByTestId("settings-section-users");
        await usersSection.waitFor();
        await search.clear();
        const credentialUser = role === "system" ? "CI Credentials Target" : "CI Credentials Organization Administrator";
        await usersSection.getByRole("row").filter({ hasText: credentialUser }).getByRole("button", { name: "Credential configuration" }).waitFor();
        if (role === "organization") assert.equal(await usersSection.getByRole("row").filter({ hasText: "CI Credentials Target" }).getByRole("button", { name: "Credential configuration" }).count(), 0, "organization administrator must not configure another user's credentials");
      }

      await search.fill("image allowlist");
      if (role === "system") await page.getByTestId("settings-section-images").waitFor();
      else assert.equal(await page.getByTestId("settings-section-images").count(), 0, "organization and member roles must not discover system settings");
      await search.fill("organization-management");
      if (role === "system") {
        const management = page.getByTestId("settings-section-organization-management");
        await management.waitFor();
        await management.getByRole("button", { name: "Create organization" }).waitFor();
        await management.getByRole("button", { name: "Delete organization" }).waitFor();
      } else {
        assert.equal(await page.getByTestId("settings-section-organization-management").count(), 0, "this fixture lacks organization-management permission");
      }
      assert.deepEqual(forbiddenRequests, [], "member Settings must not request privileged APIs");
      if (role === "system") {
        await page.getByRole("button", { name: "Operation records", exact: true }).click();
        await page.getByText("Copy API key", { exact: true }).first().waitFor();
      }
      await context.close();
    }
  } finally {
    await browser.close();
  }
});
