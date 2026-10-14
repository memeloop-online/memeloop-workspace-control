import assert from "node:assert/strict";
import { test } from "node:test";

const baseUrl = process.env.E2E_BASE_URL;
const adminToken = process.env.E2E_ADMIN_TOKEN;
const playwrightModule = process.env.E2E_PLAYWRIGHT_MODULE ?? "playwright-core";

test("system administrator edits job worker concurrency and the setting persists", { timeout: 150_000 }, async () => {
  const url = new URL(baseUrl);
  const localHttp = process.env.E2E_LOCAL_BACKEND === "1" && url.protocol === "http:" && ["127.0.0.1", "::1", "localhost"].includes(url.hostname);
  assert.ok(url.protocol === "https:" || localHttp, "E2E_BASE_URL must be HTTPS or an explicitly enabled loopback backend");
  assert.ok(adminToken, "Set E2E_ADMIN_TOKEN to an unrestricted system administrator token");

  const readScaling = async () => {
    const response = await fetch(new URL("/api/v1/admin/scaling", baseUrl), { headers: { Authorization: `Bearer ${adminToken}` } });
    assert.equal(response.status, 200, "scaling status must be readable");
    return response.json();
  };

  const original = (await readScaling()).job_worker_concurrency;
  assert.ok(Number.isSafeInteger(original) && original >= 1 && original <= 256, "scaling status must report job_worker_concurrency in the supported range");
  const next = original >= 256 ? 255 : original + 1;

  const { chromium } = await import(playwrightModule);
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_BIN || chromium.executablePath(), headless: true, args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  try {
    const context = await browser.newContext();
    await context.addInitScript((credential) => {
      sessionStorage.setItem("mwc.api-token", credential);
      localStorage.setItem("mwc.locale", "en");
      localStorage.setItem("mwc.view", "workspaces");
    }, adminToken);
    const page = await context.newPage();
    const scalingPuts = [];
    page.on("request", (request) => {
      if (request.method() === "PUT" && new URL(request.url()).pathname === "/api/v1/admin/scaling") scalingPuts.push(request.postDataJSON());
    });

    const openScalingSection = async () => {
      await page.goto(`${baseUrl}/#administration`, { waitUntil: "domcontentloaded" });
      await page.getByRole("button", { name: "Settings", exact: true }).click();
      await page.getByTestId("settings-search").fill("concurrency");
      const section = page.getByTestId("settings-section-scaling");
      await section.waitFor();
      return section;
    };

    // Invalid input is rejected locally: no PUT reaches the server and a localized error is shown.
    let section = await openScalingSection();
    await section.getByRole("button", { name: "Edit scaling" }).click();
    const editor = section.getByRole("spinbutton", { name: "Job worker concurrency" });
    await editor.fill("0");
    await section.getByRole("button", { name: "Save scaling" }).click();
    await page.locator(".fui-ToastTitle").filter({ hasText: "Job worker concurrency must be a whole number between 1 and 256." }).waitFor();
    assert.deepEqual(scalingPuts, [], "invalid input must not send a scaling PUT");
    await section.getByRole("button", { name: "Cancel" }).click();

    // A valid save sends exactly one PUT and the API read-back matches.
    await section.getByRole("button", { name: "Edit scaling" }).click();
    await editor.fill(String(next));
    const saved = page.waitForResponse((response) => response.request().method() === "PUT" && new URL(response.url()).pathname === "/api/v1/admin/scaling" && response.status() === 200);
    await section.getByRole("button", { name: "Save scaling" }).click();
    await saved;
    assert.deepEqual(scalingPuts, [{ job_worker_concurrency: next }], "the editor must send exactly one scaling PUT with the new value");
    assert.equal((await readScaling()).job_worker_concurrency, next, "API read-back must match the saved value");

    // Reloading the page keeps the persisted value.
    section = await openScalingSection();
    await section.getByRole("button", { name: "Edit scaling" }).click();
    const reloadedEditor = section.getByRole("spinbutton", { name: "Job worker concurrency" });
    await reloadedEditor.waitFor();
    assert.equal(await reloadedEditor.inputValue(), String(next), "reload must show the persisted value");
    await context.close();
  } finally {
    await browser.close();
    const response = await fetch(new URL("/api/v1/admin/scaling", baseUrl), {
      method: "PUT",
      headers: { Authorization: `Bearer ${adminToken}`, "Content-Type": "application/json", "Idempotency-Key": crypto.randomUUID() },
      body: JSON.stringify({ job_worker_concurrency: original }),
    });
    assert.equal(response.status, 200, "cleanup must restore the original job worker concurrency");
  }
});
