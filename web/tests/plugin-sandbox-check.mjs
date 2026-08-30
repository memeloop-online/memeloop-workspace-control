import { constants, access, mkdtemp, rm } from "node:fs/promises";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { chromium } from "playwright-core";

const chrome = await findChromium();
if (!chrome) {
  console.log("plugin sandbox integration: skipped (set CHROMIUM_BIN to a Chromium executable)");
  process.exit(0);
}

const server = http.createServer((request, response) => {
  if (request.url === "/parent") {
    send(response, "text/html", `<!doctype html><html><body data-plugin-result="pending" data-plugin-origin="pending">
      <iframe sandbox="allow-scripts" src="/plugin/index.html"></iframe>
      <script>addEventListener("message", event => {
        if (event.data?.type === "plugin-script-loaded") {
          document.body.dataset.pluginResult = "loaded";
          document.body.dataset.pluginOrigin = event.data.origin;
        }
      });</script>
    </body></html>`);
    return;
  }
  if (request.url === "/plugin/index.html") {
    response.setHeader("Content-Security-Policy", "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'none'; frame-ancestors 'self'; base-uri 'none'; form-action 'none'");
    send(response, "text/html", "<!doctype html><html><body><script src=\"/plugin/plugin.js\"></script></body></html>");
    return;
  }
  if (request.url === "/plugin/plugin.js") {
    send(response, "application/javascript", "parent.postMessage({type:'plugin-script-loaded',origin:self.origin}, '*');");
    return;
  }
  response.writeHead(404).end();
});

const profile = await mkdtemp(path.join(os.tmpdir(), "mwc-plugin-sandbox-"));
let browser;
try {
  const port = await listen(server);
  browser = await chromium.launch({ executablePath: chrome, headless: true, args: ["--no-sandbox", "--disable-dev-shm-usage"], timeout: 15_000 });
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(`http://127.0.0.1:${port}/parent`, { waitUntil: "load", timeout: 10_000 });
  await page.waitForFunction(() => document.body.dataset.pluginResult === "loaded", undefined, { timeout: 5_000 });
  const pluginOrigin = await page.evaluate(() => document.body.dataset.pluginOrigin);
  if (pluginOrigin !== "null") throw new Error(`plugin iframe origin was not opaque: ${pluginOrigin}`);
  console.log("plugin sandbox integration: external plugin script loaded in an opaque-origin iframe");
} finally {
  await browser?.close();
  server.close();
  await rm(profile, { recursive: true, force: true });
}

function send(response, contentType, body) {
  response.writeHead(200, { "Content-Type": contentType, "Cache-Control": "no-store" });
  response.end(body);
}

function listen(instance) {
  return new Promise((resolve, reject) => {
    instance.once("error", reject);
    instance.listen(0, "127.0.0.1", () => resolve(instance.address().port));
  });
}

async function findChromium() {
  const candidates = [
    process.env.CHROMIUM_BIN,
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    "/home/token-center-dev/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome",
  ].filter(Boolean);
  for (const candidate of candidates) {
    try {
      await access(candidate, constants.X_OK);
      return candidate;
    } catch {
      // Continue to the next well-known executable.
    }
  }
  return null;
}
