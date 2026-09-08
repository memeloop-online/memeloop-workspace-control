import { chromium } from "playwright-core";
import { mkdir } from "node:fs/promises";

const outputDirectory = "/home/token-center-dev/.codex/visualizations/2026/08/25/01a03b21-c07f-7713-864f-f29b10d74a6f/mwc-sandbox-key-ui";
const executablePath = "/home/token-center-dev/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome";
const browser = await chromium.launch({ executablePath, headless: true });
await mkdir(outputDirectory, { recursive: true });

for (const width of [360, 768, 1440]) {
  const page = await browser.newPage({ viewport: { width, height: 900 }, deviceScaleFactor: 1 });
  await page.goto(`http://127.0.0.1:5173/tests/api-key-ui-fixture.html?restricted=1`, { waitUntil: "networkidle" });
  const formState = await page.locator("form").evaluate((form) => {
    const picker = form.querySelector(".api-key-template-picker");
    const actions = form.querySelector(".api-key-create-actions");
    const restriction = form.querySelector(".api-key-template-toggle input");
    return {
      pickerBeforeActions: Boolean(picker && actions && picker.compareDocumentPosition(actions) === Node.DOCUMENT_POSITION_FOLLOWING),
      restrictionLocked: restriction?.disabled === true,
      fullWidth: document.documentElement.scrollWidth <= window.innerWidth,
    };
  });
  if (!formState.pickerBeforeActions || !formState.restrictionLocked || !formState.fullWidth) {
    throw new Error(`API key template fixture failed at ${width}px: ${JSON.stringify(formState)}`);
  }
  await page.getByRole("button", { name: /choose a template|选择模板|выбрать шаблон/i }).click();
  const optionsState = await page.locator(".api-key-template-options").evaluate((options) => ({
    scrollable: options.scrollHeight > options.clientHeight && getComputedStyle(options).overflowY === "auto",
    fullUuidVisible: options.textContent?.includes("00000000-0000-4000-8000-") === true,
  }));
  if (!optionsState.scrollable || optionsState.fullUuidVisible) {
    throw new Error(`API key template options fixture failed at ${width}px: ${JSON.stringify(optionsState)}`);
  }
  await page.screenshot({ path: `${outputDirectory}/api-key-template-${width}.png`, fullPage: true });
  await page.close();
}

await browser.close();
