import assert from "node:assert/strict";
import { readFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

const baseUrl = process.env.E2E_BASE_URL;
const adminToken = process.env.E2E_ADMIN_TOKEN;
const playwrightModule = process.env.E2E_PLAYWRIGHT_MODULE ?? "playwright-core";
const translations = Object.fromEntries(["en", "zh-CN", "ru"].map((locale) => [locale, JSON.parse(readFileSync(new URL(`../src/locales/${locale}.json`, import.meta.url), "utf8"))]));

test("administrator saves CPU overcommit and concurrency independently with localized validation", { timeout: 150_000 }, async () => {
  const url = new URL(baseUrl);
  const localHttp = process.env.E2E_LOCAL_BACKEND === "1" && url.protocol === "http:" && ["127.0.0.1", "::1", "localhost"].includes(url.hostname);
  assert.ok(url.protocol === "https:" || localHttp, "E2E_BASE_URL must be HTTPS or an explicitly enabled loopback backend");
  assert.ok(adminToken, "Set E2E_ADMIN_TOKEN to an unrestricted system administrator token");
  const headers = { Authorization: `Bearer ${adminToken}`, "Content-Type": "application/json" };
  const readScaling = async () => {
    const response = await fetch(new URL("/api/v1/admin/scaling", baseUrl), { headers });
    assert.equal(response.status, 200);
    return response.json();
  };
  const writeScaling = async (patch) => {
    const response = await fetch(new URL("/api/v1/admin/scaling", baseUrl), { method: "PUT", headers: { ...headers, "Idempotency-Key": crypto.randomUUID() }, body: JSON.stringify(patch) });
    assert.equal(response.status, 200);
  };
  const original = await readScaling();
  assert.ok(Number.isSafeInteger(original.job_worker_concurrency) && original.job_worker_concurrency >= 1 && original.job_worker_concurrency <= 256);
  assert.ok(Number.isSafeInteger(original.cpu_overcommit_ratio) && original.cpu_overcommit_ratio >= 1 && original.cpu_overcommit_ratio <= 1000);
  const nextConcurrency = original.job_worker_concurrency === 256 ? 255 : original.job_worker_concurrency + 1;
  const nextRatio = original.cpu_overcommit_ratio === 1000 ? 999 : original.cpu_overcommit_ratio + 1;
  const { chromium } = await import(playwrightModule);
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_BIN || chromium.executablePath(), headless: true, args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  try {
    const context = await browser.newContext();
    await context.addInitScript((credential) => {
      sessionStorage.setItem("mwc.api-token", credential);
      if (!localStorage.getItem("mwc.locale")) localStorage.setItem("mwc.locale", "en");
      localStorage.setItem("mwc.view", "workspaces");
    }, adminToken);
    const page = await context.newPage();
    const scalingPuts = [];
    page.on("request", (request) => {
      if (request.method() === "PUT" && new URL(request.url()).pathname === "/api/v1/admin/scaling") scalingPuts.push(request.postDataJSON());
    });
    const openScaling = async (locale = "en") => {
      const copy = translations[locale];
      await page.goto(`${baseUrl}/#administration`, { waitUntil: "domcontentloaded" });
      await page.reload({ waitUntil: "domcontentloaded" });
      await page.getByRole("button", { name: copy.settingsTitle, exact: true }).click();
      await page.getByTestId("settings-search").fill(copy.cpuOvercommitRatio);
      const section = page.getByTestId("settings-section-scaling");
      await section.waitFor();
      return section;
    };
    const edit = async (section, locale = "en") => {
      await section.getByRole("button", { name: translations[locale].editScaling }).click();
      return {
        cpu: section.getByRole("spinbutton", { name: translations[locale].cpuOvercommitRatio }),
        concurrency: section.getByRole("spinbutton", { name: translations[locale].jobWorkerConcurrency }),
      };
    };
    const save = async (section, locale = "en") => {
      const saved = page.waitForResponse((response) => response.request().method() === "PUT" && new URL(response.url()).pathname === "/api/v1/admin/scaling" && response.status() === 200);
      await section.getByRole("button", { name: translations[locale].saveScaling }).click();
      await saved;
      await section.getByRole("button", { name: translations[locale].editScaling }).waitFor();
    };
    let section = await openScaling();
    let editor = await edit(section);
    await editor.concurrency.fill("0");
    await section.getByRole("button", { name: translations.en.saveScaling }).click();
    await page.locator(".fui-ToastTitle").filter({ hasText: translations.en.invalidJobWorkerConcurrency }).waitFor();
    assert.deepEqual(scalingPuts, []);
    await section.getByRole("button", { name: translations.en.cancel }).click();

    // Validate and review the existing-style CPU field in all supported languages.
    for (const locale of ["en", "zh-CN", "ru"]) {
      if (locale !== "en") {
        await page.evaluate((value) => localStorage.setItem("mwc.locale", value), locale);
        section = await openScaling(locale);
      }
      editor = await edit(section, locale);
      assert.equal(await editor.cpu.inputValue(), String(original.cpu_overcommit_ratio));
      assert.equal(await editor.cpu.getAttribute("min"), "1");
      assert.equal(await editor.cpu.getAttribute("max"), "1000");
      await section.getByText(translations[locale].cpuOvercommitRatioHelp, { exact: true }).waitFor();
      await editor.cpu.fill("1001");
      await section.getByRole("button", { name: translations[locale].saveScaling }).click();
      await page.locator(".fui-ToastTitle").filter({ hasText: translations[locale].invalidCpuOvercommitRatio }).waitFor();
      assert.deepEqual(scalingPuts, []);
      if (process.env.E2E_LOG_DIR) {
        mkdirSync(process.env.E2E_LOG_DIR, { recursive: true });
        await page.screenshot({ path: join(process.env.E2E_LOG_DIR, `cpu-overcommit-${locale}.png`), fullPage: true });
      }
      await section.getByRole("button", { name: translations[locale].cancel }).click();
    }
    await page.evaluate(() => localStorage.setItem("mwc.locale", "en"));
    section = await openScaling();
    editor = await edit(section);
    await editor.cpu.fill(String(nextRatio));
    await section.getByRole("button", { name: translations.en.cancel }).click();
    editor = await edit(section);
    assert.equal(await editor.cpu.inputValue(), String(original.cpu_overcommit_ratio), "cancel must discard draft");
    await section.getByRole("button", { name: translations.en.saveScaling }).click();
    await section.getByRole("button", { name: translations.en.editScaling }).waitFor();
    assert.deepEqual(scalingPuts, [], "unchanged save sends no empty patch");

    // Another administrator changes concurrency after the editor opened.
    editor = await edit(section);
    await writeScaling({ job_worker_concurrency: nextConcurrency });
    await editor.cpu.fill(String(nextRatio));
    await save(section);
    assert.deepEqual(scalingPuts, [{ cpu_overcommit_ratio: nextRatio }]);
    let actual = await readScaling();
    assert.equal(actual.cpu_overcommit_ratio, nextRatio);
    assert.equal(actual.job_worker_concurrency, nextConcurrency, "CPU-only save preserves concurrent administrator changes");

    section = await openScaling();
    editor = await edit(section);
    assert.equal(await editor.cpu.inputValue(), String(nextRatio), "reload must show persisted CPU ratio");
    await editor.concurrency.fill(String(original.job_worker_concurrency));
    await save(section);
    assert.deepEqual(scalingPuts.at(-1), { job_worker_concurrency: original.job_worker_concurrency });
    assert.equal((await readScaling()).cpu_overcommit_ratio, nextRatio);

    // A server failure retains both drafts and re-enables retry/cancel.
    editor = await edit(section);
    await editor.cpu.fill(String(original.cpu_overcommit_ratio));
    await editor.concurrency.fill(String(nextConcurrency));
    let releaseFailure;
    const failureGate = new Promise((resolve) => { releaseFailure = resolve; });
    await page.route("**/api/v1/admin/scaling", async (route) => {
      if (route.request().method() !== "PUT") return route.continue();
      await failureGate;
      await route.fulfill({ status: 503, contentType: "application/json", body: JSON.stringify({ error: { code: "unavailable", message: "Scaling save unavailable" } }) });
    });
    await section.getByRole("button", { name: translations.en.saveScaling }).click();
    await section.getByRole("button", { name: translations.en.saving, exact: true }).waitFor();
    assert.equal(await editor.cpu.isDisabled(), true);
    assert.equal(await editor.concurrency.isDisabled(), true);
    assert.equal(await section.getByRole("button", { name: translations.en.cancel }).isDisabled(), true);
    releaseFailure();
    await page.locator(".fui-ToastTitle").filter({ hasText: "Scaling save unavailable" }).waitFor();
    assert.equal(await editor.cpu.inputValue(), String(original.cpu_overcommit_ratio));
    assert.equal(await editor.concurrency.inputValue(), String(nextConcurrency));
    assert.equal(await editor.cpu.isEnabled(), true);
    assert.equal(await section.getByRole("button", { name: translations.en.cancel }).isEnabled(), true);
    await page.unroute("**/api/v1/admin/scaling");
    await save(section);
    assert.deepEqual(scalingPuts.at(-1), { job_worker_concurrency: nextConcurrency, cpu_overcommit_ratio: original.cpu_overcommit_ratio });
    actual = await readScaling();
    assert.equal(actual.job_worker_concurrency, nextConcurrency);
    assert.equal(actual.cpu_overcommit_ratio, original.cpu_overcommit_ratio);
    await context.close();
  } finally {
    await browser.close();
    await writeScaling({ job_worker_concurrency: original.job_worker_concurrency, cpu_overcommit_ratio: original.cpu_overcommit_ratio });
  }
});
