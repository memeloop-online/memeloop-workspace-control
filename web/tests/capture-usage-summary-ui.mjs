import { chromium } from "playwright-core";
import { constants, access, mkdir } from "node:fs/promises";
import { resolve } from "node:path";

const outputDirectory = process.env.UI_FIXTURE_OUTPUT_DIR ?? resolve("artifacts/ui-review/usage-summary");
const executablePath = await findChromium();
const origin = process.env.UI_FIXTURE_ORIGIN ?? "http://127.0.0.1:5173";
if (!executablePath) throw new Error("No Chromium executable found; set CHROMIUM_BIN");
const browser = await chromium.launch({ executablePath, headless: true });
await mkdir(outputDirectory, { recursive: true });
for (const width of [360, 768, 1440]) {
  const page = await browser.newPage({ viewport: { width, height: 900 }, deviceScaleFactor: 1 });
  await page.goto(`${origin}/tests/usage-summary-fixture.html`, { waitUntil: "networkidle" });
  const state = await page.locator(".workspace-stats").evaluate((stats) => ({ cards: stats.children.length, fullWidth: document.documentElement.scrollWidth <= window.innerWidth, bars: stats.querySelectorAll(".workspace-stat-fill").length }));
  if (state.cards !== 4 || !state.fullWidth || state.bars !== 3) throw new Error(`Usage summary fixture failed at ${width}px: ${JSON.stringify(state)}`);
  await page.screenshot({ path: `${outputDirectory}/usage-summary-${width}.png`, fullPage: true });
  await page.close();
}
await browser.close();

async function findChromium() {
  const candidates = [process.env.CHROMIUM_BIN, "/usr/bin/google-chrome", "/usr/bin/chromium", "/usr/bin/chromium-browser", chromium.executablePath()].filter(Boolean);
  for (const candidate of candidates) {
    try { await access(candidate, constants.X_OK); return candidate; } catch { /* try next browser */ }
  }
  return null;
}
