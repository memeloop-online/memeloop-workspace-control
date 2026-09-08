import assert from "node:assert/strict";
import { createServer } from "node:http";
import test from "node:test";
import { chromium } from "../web/node_modules/playwright-core/index.mjs";
import { attachSocketEvidence, findChromium } from "./web-shell-browser-acceptance-browser.ts";

class FakeCdp {
  private handlers = new Map<string, (event: any) => void>();
  on(name: string, handler: (event: any) => void): void { this.handlers.set(name, handler); }
  emit(name: string, event: any): void { this.handlers.get(name)?.(event); }
}

function emitHandshake(cdp: FakeCdp, requestId: string, status: number, ticket: string): void {
  cdp.emit("Network.webSocketCreated", {
    requestId, url: `wss://shell.test/shell/demo/ws?ticket=${ticket}`,
  });
  // The request event intentionally has no URL: CDP identifies it by requestId.
  cdp.emit("Network.webSocketWillSendHandshakeRequest", { requestId, request: { headers: {} } });
  // The response event intentionally has no URL either.
  cdp.emit("Network.webSocketHandshakeResponseReceived", { requestId, response: { status } });
}

test("CDP fixture proves a successful ttyd handshake and frames", () => {
  const cdp = new FakeCdp();
  const evidence = attachSocketEvidence(cdp, "/shell/demo/ws");
  emitHandshake(cdp, "accepted", 101, "fixture-accepted");
  cdp.emit("Network.webSocketFrameSent", { requestId: "accepted" });
  cdp.emit("Network.webSocketFrameReceived", { requestId: "accepted" });
  cdp.emit("Network.webSocketCreated", { requestId: "other", url: "wss://shell.test/other/ws" });
  cdp.emit("Network.webSocketWillSendHandshakeRequest", { requestId: "other", request: {} });
  assert.deepEqual(evidence, { requestCount: 1, statuses: [101], frameCount: 2, frameErrorStatuses: [] });
});

test("CDP fixture proves replay is rejected by HTTP status", () => {
  const cdp = new FakeCdp();
  const evidence = attachSocketEvidence(cdp, "/shell/demo/ws");
  emitHandshake(cdp, "replayed", 401, "fixture-replayed");
  assert.equal(evidence.requestCount, 1);
  assert.deepEqual(evidence.statuses, [401]);
  assert.deepEqual(evidence.frameErrorStatuses, []);
  assert.equal(evidence.frameCount, 0);
  assert.equal(evidence.statuses.some((status) => status === 401 || status === 403), true);
});

test("real Chromium fixture normalizes a 401 WebSocket frame error", { timeout: 15_000 }, async () => {
  let upgradeSeen = false;
  const server = createServer((request, response) => {
    if (request.url?.startsWith("/shell/demo/ws") && request.headers.upgrade?.toLowerCase() === "websocket") {
      upgradeSeen = true;
      response.writeHead(401, { connection: "close", "content-length": "0" });
      response.end();
      return;
    }
    response.writeHead(200, { "content-type": "text/html" });
    response.end("<script>new WebSocket(`ws://${location.host}/shell/demo/ws?ticket=fixture`).onerror=()=>{}</script>");
  });
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => resolve());
  });
  let browser: any;
  let context: any;
  try {
    const address = server.address();
    assert.ok(address && typeof address !== "string");
    browser = await chromium.launch({ executablePath: await findChromium(), headless: true, args: ["--no-sandbox"] });
    context = await browser.newContext();
    const page = await context.newPage();
    const cdp = await context.newCDPSession(page);
    await cdp.send("Network.enable");
    const evidence = attachSocketEvidence(cdp, "/shell/demo/ws");
    await page.goto(`http://127.0.0.1:${address.port}/`, { waitUntil: "domcontentloaded" });
    const deadline = Date.now() + 5_000;
    while (Date.now() < deadline && !evidence.frameErrorStatuses.includes(401)) {
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 50));
    }
    assert.equal(upgradeSeen, true);
    assert.equal(evidence.requestCount, 1);
    assert.deepEqual(evidence.frameErrorStatuses, [401]);
    assert.deepEqual(evidence.statuses, [401]);
  } finally {
    if (context) await context.close().catch(() => undefined);
    if (browser) await browser.close().catch(() => undefined);
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }
});
