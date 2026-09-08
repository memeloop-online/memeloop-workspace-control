import assert from "node:assert/strict";
import test from "node:test";
import { attachSocketEvidence } from "./web-shell-browser-acceptance-browser.ts";

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
  assert.deepEqual(evidence, { requestCount: 1, statuses: [101], frameCount: 2 });
});

test("CDP fixture proves replay is rejected by HTTP status", () => {
  const cdp = new FakeCdp();
  const evidence = attachSocketEvidence(cdp, "/shell/demo/ws");
  emitHandshake(cdp, "replayed", 401, "fixture-replayed");
  assert.equal(evidence.requestCount, 1);
  assert.deepEqual(evidence.statuses, [401]);
  assert.equal(evidence.frameCount, 0);
  assert.equal(evidence.statuses.some((status) => status === 401 || status === 403), true);
});
