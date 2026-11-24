import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { chromium } from "playwright-core";

const baseUrl = process.env.E2E_BASE_URL ?? "http://127.0.0.1:4173";
const messages = JSON.parse(await readFile(new URL("../src/locales/en.json", import.meta.url), "utf8"));
const proofKey = "fixture-current-full-permission-key-0123456789";
const bearerKey = "fixture-independent-api-key-0123456789";
const principal = {
  user_id: "same-account",
  display_name: "Fixture Account",
  system_admin: false,
  memberships: [],
  api_key_scopes: ["read_workspace"],
  api_key_expires_at: null,
  allowed_template_ids: null,
};
const identityOrigin = "https://identity.example.test";

test("single sign-on links with a separate proof, uses a cookie, and keeps API key sign-in independent", { timeout: 60_000 }, async () => {
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_BIN || chromium.executablePath(), headless: true, args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  const events = [];
  const unexpected = [];
  let linked = false;
  let signedIn = false;
  try {
    const context = await browser.newContext();
    await context.addInitScript(() => localStorage.setItem("mwc.locale", "en"));
    await context.route("**/*", async (route) => {
      const request = route.request();
      const url = new URL(request.url());
      const method = request.method();
      const auth = request.headers().authorization;
      const path = url.pathname;
      if (url.origin === identityOrigin) {
        events.push({ type: "provider", auth, search: url.search });
        const action = url.searchParams.get("action");
        if (action === "login" && linked) signedIn = true;
        return route.fulfill({ status: 302, headers: { location: new URL(action === "login" ? (linked ? "/" : "/?oidc=unbound") : `/?oidc=${action}`, baseUrl).href } });
      }
      if (url.origin !== new URL(baseUrl).origin) return route.abort();
      if (!path.startsWith("/api/")) return route.continue();

      if (method === "GET" && path === "/api/v1/auth/oidc/status") {
        return route.fulfill({ json: { enabled: true, signed_in: signedIn } });
      }
      if (method === "POST" && path === "/api/v1/auth/oidc/start") {
        const { action } = request.postDataJSON();
        events.push({ type: "start", action, auth });
        return route.fulfill({ json: { authorization_url: `${identityOrigin}/authorize?action=${action}` } });
      }
      if (method === "POST" && path === "/api/v1/auth/oidc/confirm") {
        const action = request.postDataJSON().action;
        events.push({ type: "confirm", action, auth, search: url.search });
        if (auth !== `Bearer ${proofKey}`) return route.fulfill({ status: 403 });
        if (action === "bind") linked = true;
        if (action === "unbind") linked = signedIn = false;
        return route.fulfill({ status: 204 });
      }
      if (method === "POST" && path === "/api/v1/auth/oidc/logout") {
        events.push({ type: "logout", auth });
        signedIn = false;
        return route.fulfill({ status: 204 });
      }
      if (method === "GET" && (auth === `Bearer ${bearerKey}` || (signedIn && auth === undefined))) {
        if (path === "/api/v1/me") {
          events.push({ type: "principal", auth });
          return route.fulfill({ json: principal });
        }
        if (path === "/api/v1/organizations") return route.fulfill({ json: { items: [], next_cursor: null } });
        if (path === "/api/v1/me/profile") return route.fulfill({ json: { display_name: principal.display_name, avatar_url: null } });
      }
      unexpected.push(`${method} ${path}`);
      return route.fulfill({ status: 501, json: {} });
    });
    const page = await context.newPage();
    await page.goto(baseUrl);
    const button = (key) => page.getByRole("button", { name: messages[key], exact: true });
    const userMenu = page.getByRole("button", { name: /Fixture Account/ });
    const menuItem = (key) => page.getByRole("menuitem", { name: messages[key], exact: true });
    const proofInput = page.getByLabel(messages.ssoAccountKey, { exact: true });
    await button("ssoSignIn").waitFor();
    assert.deepEqual(events, [], "merely visiting the page does not start a provider flow");
    await button("ssoSignIn").click();
    await page.getByText(messages.ssoAccountNotLinked).waitFor();
    await button("ssoBind").click();
    await proofInput.fill("fixture-restricted-api-key-0123456789");
    await button("ssoConfirm").click();
    await page.getByText(messages.ssoProofRejected).waitFor();
    assert.equal(await proofInput.inputValue(), "");
    await proofInput.fill(proofKey);
    await button("ssoConfirm").click();
    await page.getByText(messages.ssoLinked).waitFor();
    assert.equal(new URL(page.url()).search, "");
    assert.equal(await page.evaluate(() => sessionStorage.getItem("mwc.api-token")), null);
    assert.equal(await userMenu.count(), 0, "linking alone must not sign the user in");

    await button("ssoSignIn").click();
    await userMenu.waitFor();
    assert.deepEqual(events.filter(({ type }) => type === "principal").at(-1), { type: "principal", auth: undefined });
    await userMenu.click();
    await menuItem("logout").click();
    await button("ssoSignIn").waitFor();
    assert.deepEqual(events.filter(({ type }) => type === "logout"), [{ type: "logout", auth: undefined }]);

    await page.getByLabel(messages.token, { exact: true }).fill(bearerKey);
    await button("ssoSignInWithKey").click();
    await userMenu.waitFor();
    signedIn = true; // A cookie also exists; the explicit API key choice still wins after reload.
    await page.reload();
    await userMenu.waitFor();
    assert.deepEqual(events.filter(({ type }) => type === "principal").at(-1), { type: "principal", auth: `Bearer ${bearerKey}` });
    signedIn = false;
    await userMenu.click();
    await menuItem("ssoUnbind").click();
    await proofInput.fill(proofKey);
    await button("ssoConfirm").click();
    await page.getByText(messages.ssoUnlinked).waitFor();
    assert.equal(await page.evaluate(() => sessionStorage.getItem("mwc.api-token")), bearerKey, "proof does not replace the signed-in key");
    assert.ok(events.filter(({ type }) => type === "start" || type === "provider").every(({ auth }) => auth === undefined));
    assert.ok(events.filter(({ type }) => type === "confirm").every(({ search }) => search === ""));
    assert.equal(linked, false);
    assert.deepEqual(unexpected, []);
  } finally {
    await browser.close();
  }
});
