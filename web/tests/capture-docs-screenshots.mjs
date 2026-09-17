import { chromium } from "playwright-core";
import { mkdir } from "node:fs/promises";
import path from "node:path";

const token = process.env.MWC_SCREENSHOT_TOKEN;
const origin = process.env.MWC_SCREENSHOT_ORIGIN;
const output = path.resolve(process.env.MWC_SCREENSHOT_OUTPUT ?? "../docs-screenshots");

if (!token || !origin) throw new Error("Screenshot credentials and origin are required");
await mkdir(output, { recursive: true });

const browser = await chromium.launch({ headless: true });

async function openProduct(viewport, hash) {
  const context = await browser.newContext({
    viewport,
    deviceScaleFactor: 1,
    colorScheme: "light",
    locale: "zh-CN",
  });
  const page = await context.newPage();
  await page.addInitScript(({ value }) => {
    sessionStorage.setItem("mwc.api-token", value);
    localStorage.setItem("mwc.locale", "zh-CN");
    localStorage.setItem("mwc.theme", "light");
  }, { value: token });
  await page.goto(`${origin}/#${hash}`, { waitUntil: "domcontentloaded", timeout: 60_000 });
  await page.locator("main").waitFor({ state: "visible", timeout: 30_000 });
  await page.waitForTimeout(2_000);
  return { context, page };
}

async function sanitize(page) {
  await page.evaluate(() => {
    const replacements = [
      [/\b[\w.+-]+@[\w.-]+\.[A-Za-z]{2,}\b/g, "user@example.com"],
      [/\bw-[a-f0-9]{16}\b/gi, "w-example000000000"],
      [/\b[a-f0-9]{16}\b/gi, "example000000000"],
      [/\b01[a-z0-9]{6,}\b/gi, "01example"],
      [/\b[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\b/gi, "00000000-0000-4000-8000-000000000000"],
      [/\blindongwu11\b/gi, "示例用户"],
      [/\btoken-center-dev\b/gi, "rust-dev-demo"],
      [/\bmaintainance\b/gi, "maintenance-demo"],
      [/\bgame-forking\b/gi, "node-dev-demo"],
      [/\btiddlywiki-dev\b/gi, "web-dev-demo"],
      [/\brust-dev-test\b/gi, "rust-dev-test"],
      [/personal-codex-agent-codex53/gi, "example-agent-profile"],
      [/personal-codex-agent-kimi/gi, "example-model-profile"],
      [/personal-codex-agent-luna/gi, "example-worker-profile"],
      [/personal-codex-auth/gi, "example-service-token"],
      [/personal-codex-config/gi, "example-tool-config"],
      [/personal-codex-env/gi, "example-environment"],
      [/personal-gh-config/gi, "example-git-config"],
      [/personal-gh-device-id/gi, "example-device-profile"],
    ];
    const replace = (input) => {
      let value = input;
      for (const [pattern, replacement] of replacements) value = value.replace(pattern, replacement);
      return value;
    };
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      node.textContent = replace(node.textContent ?? "");
    }
    document.querySelectorAll("input, textarea").forEach((element) => {
      if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
        element.value = replace(element.value);
        if (/token|密钥|凭据|value|值/i.test(`${element.name} ${element.placeholder} ${element.getAttribute("aria-label") ?? ""}`)) {
          element.value = "";
        }
      }
    });
    const style = document.createElement("style");
    style.textContent = "*,*::before,*::after{animation:none!important;transition:none!important;caret-color:transparent!important}";
    document.head.append(style);
  });
}

async function capture(viewport, hash, filename, { fullPage = false, fitFirstCard = false } = {}) {
  const { context, page } = await openProduct(viewport, hash);
  await sanitize(page);
  if (fitFirstCard) {
    const firstTitle = page.locator('h2[id^="workspace-"][id$="-title"]').first();
    await firstTitle.waitFor({ state: "visible", timeout: 30_000 });
    const firstCard = firstTitle.locator(
      'xpath=ancestor::*[contains(concat(" ", normalize-space(@class), " "), " fui-Card ")][1]',
    );
    for (let attempt = 0; attempt < 3; attempt++) {
      const box = await firstCard.boundingBox();
      const size = page.viewportSize();
      if (!box || !size) break;
      const requiredHeight = Math.ceil(box.y + box.height) + 24;
      if (requiredHeight <= size.height) break;
      await page.setViewportSize({ width: size.width, height: requiredHeight });
      await page.waitForTimeout(300);
    }
  }
  await page.screenshot({ path: path.join(output, filename), fullPage });
  await context.close();
}

try {
  await capture({ width: 1440, height: 1000 }, "workspaces", "workspaces-desktop.png");
  await capture({ width: 390, height: 844 }, "workspaces", "workspaces-mobile.png", { fitFirstCard: true });
  await capture({ width: 1440, height: 1000 }, "injections", "credentials-desktop.png");
} finally {
  await browser.close();
}
