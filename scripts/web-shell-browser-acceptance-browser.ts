import { randomBytes } from "node:crypto";
import { access, readdir } from "node:fs/promises";
import { constants as fsConstants } from "node:fs";

export class AcceptanceError extends Error {
  readonly stage: string;
  constructor(stage: string, message: string) {
    super(message); this.name = "AcceptanceError"; this.stage = stage;
  }
}
const fail = (stage: string, message: string): never => { throw new AcceptanceError(stage, message); };
export type BrowserTicket = { ticket: string; url: URL };
type SurfaceState = {
  hasTerm: boolean; hasBuffer: boolean; hasTextarea: boolean;
  termCols: number | null; termRows: number | null;
};
type SocketEvidence = {
  requestCount: number; statuses: number[]; frameCount: number;
  frameErrorStatuses: Array<401 | 403>;
};
type Session = { page: any; evidence: SocketEvidence };
const pause = (milliseconds: number) => new Promise<void>((resolvePromise) => setTimeout(resolvePromise, milliseconds));

function normalizedPath(pathname: string): string { return pathname.replace(/\/+$/, "") || "/"; }
function expectedSocketPath(ticketUrl: URL): string { return `${normalizedPath(ticketUrl.pathname)}/ws`; }
function socketPathMatches(value: unknown, expectedPath: string): boolean {
  if (typeof value !== "string") return false;
  try {
    const parsed = new URL(value);
    return (parsed.protocol === "ws:" || parsed.protocol === "wss:") && normalizedPath(parsed.pathname) === normalizedPath(expectedPath);
  } catch { return false; }
}

/** CDP's Created event is the URL-bearing event; later events only carry requestId. */
export function attachSocketEvidence(cdp: any, expectedPath: string): SocketEvidence {
  const evidence: SocketEvidence = { requestCount: 0, statuses: [], frameCount: 0, frameErrorStatuses: [] };
  const createdIds = new Set<string>(); const handshakeIds = new Set<string>();
  cdp.on("Network.webSocketCreated", (event: any) => {
    if (typeof event?.requestId === "string" && socketPathMatches(event.url, expectedPath)) createdIds.add(event.requestId);
  });
  cdp.on("Network.webSocketWillSendHandshakeRequest", (event: any) => {
    const id = event?.requestId;
    if (typeof id === "string" && createdIds.has(id) && !handshakeIds.has(id)) {
      handshakeIds.add(id); evidence.requestCount += 1;
    }
  });
  cdp.on("Network.webSocketHandshakeResponseReceived", (event: any) => {
    const id = event?.requestId;
    if (typeof id !== "string" || !handshakeIds.has(id)) return;
    const status = Number(event?.response?.status);
    if (Number.isInteger(status) && evidence.statuses.length < 32) evidence.statuses.push(status);
  });
  cdp.on("Network.webSocketFrameError", (event: any) => {
    if (!handshakeIds.has(event?.requestId)) return;
    const message = typeof event?.errorMessage === "string" ? event.errorMessage : "";
    // Chromium reports HTTP 401 through this canonical auth failure instead of
    // exposing a WebSocket handshake response event. Keep only the status code.
    const unexpected = /unexpected response code\s*:\s*(401|403)\b/i.exec(message);
    const status = unexpected ? Number(unexpected[1]) : message.trim() === "HTTP Authentication failed; no valid credentials available" ? 401 : 0;
    if (status !== 401 && status !== 403) return;
    if (evidence.statuses.length < 32) evidence.statuses.push(status);
    if (evidence.frameErrorStatuses.length < 32) evidence.frameErrorStatuses.push(status);
  });
  const countFrame = (event: any) => { if (handshakeIds.has(event?.requestId)) evidence.frameCount += 1; };
  cdp.on("Network.webSocketFrameSent", countFrame);
  cdp.on("Network.webSocketFrameReceived", countFrame);
  return evidence;
}

async function browserStep<T>(stage: string, action: () => Promise<T>): Promise<T> {
  try { return await action(); } catch (error) {
    if (error instanceof AcceptanceError) throw error;
    fail(stage, "browser operation failed");
  }
}
export async function findChromium(): Promise<string> {
  const cacheRoot = "/home/token-center-dev/.cache/ms-playwright";
  const candidates = [process.env.CHROMIUM_BIN, "/usr/bin/google-chrome", "/usr/bin/chromium", "/usr/bin/chromium-browser"]
    .filter((value): value is string => Boolean(value));
  try {
    for (const entry of await readdir(cacheRoot, { withFileTypes: true })) {
      if (entry.isDirectory() && entry.name.startsWith("chromium-")) candidates.push(`${cacheRoot}/${entry.name}/chrome-linux64/chrome`);
    }
  } catch { /* fixed system candidates may still work */ }
  for (const candidate of candidates) {
    try { await access(candidate, fsConstants.X_OK); return candidate; } catch { /* try the next local executable */ }
  }
  fail("browser", "a local Chromium executable was not found");
}

async function openTicketPage(context: any, pages: Set<any>, ticket: BrowserTicket, timeoutMs: number): Promise<Session> {
  return browserStep("browser", async () => {
    const page = await context.newPage(); pages.add(page);
    try {
      const cdp = await context.newCDPSession(page); await cdp.send("Network.enable");
      const evidence = attachSocketEvidence(cdp, expectedSocketPath(ticket.url));
      const response = await page.goto(ticket.url.href, { waitUntil: "domcontentloaded", timeout: timeoutMs });
      const status = response?.status();
      if (!status || status < 200 || status >= 300) {
        fail("browser", `Web Shell page returned HTTP ${status ?? "unavailable"}`);
      }
      return { page, evidence };
    } catch (error) {
      await page.close().catch(() => undefined); pages.delete(page);
      if (error instanceof AcceptanceError) throw error;
      fail("browser", "Web Shell page could not be opened");
    }
  });
}

async function readSurfaceState(page: any): Promise<SurfaceState> {
  return browserStep("terminal", () => page.evaluate(() => {
    const term = window.term; const buffer = term?.buffer?.active;
    const input = document.querySelector(".xterm-helper-textarea");
    const numberOrNull = (value: unknown) => typeof value === "number" && Number.isFinite(value) ? value : null;
    return {
      hasTerm: Boolean(term), hasBuffer: Boolean(buffer && typeof buffer.getLine === "function"), hasTextarea: Boolean(input),
      termCols: numberOrNull(term?.cols), termRows: numberOrNull(term?.rows),
    };
  }));
}

async function terminalHasText(page: any, needle: string): Promise<boolean> {
  return browserStep("terminal", () => page.evaluate((expected) => {
    const active = window.term?.buffer?.active;
    if (!active || typeof active.getLine !== "function") return false;
    let tail = ""; const limit = expected.length + 2;
    for (let index = 0; index < active.length; index += 1) {
      const line = active.getLine(index)?.translateToString(true) ?? "";
      tail = (tail + line).slice(-limit);
      if (tail.includes(expected)) return true;
    }
    return false;
  }, needle));
}

async function terminalSize(page: any, begin: string, end: string): Promise<{ rows: number; cols: number } | null> {
  return browserStep("terminal", () => page.evaluate(({ begin: start, end: finish }) => {
    const active = window.term?.buffer?.active;
    if (!active || typeof active.getLine !== "function") return null;
    let tail = ""; const limit = start.length + finish.length + 80;
    for (let index = 0; index < active.length; index += 1) {
      tail = (tail + (active.getLine(index)?.translateToString(true) ?? "")).slice(-limit);
      const beginAt = tail.lastIndexOf(start); const endAt = tail.indexOf(finish, beginAt + start.length);
      if (beginAt < 0 || endAt < 0) continue;
      const match = tail.slice(beginAt + start.length, endAt).match(/^\s*(\d+)\s+(\d+)\s*$/);
      if (match) return { rows: Number(match[1]), cols: Number(match[2]) };
    }
    return null;
  }, { begin, end }));
}

async function waitForEvidence(evidence: SocketEvidence, predicate: (value: SocketEvidence) => boolean, timeoutMs: number, stage: string): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) { if (predicate(evidence)) return; await pause(100); }
  fail(stage, "expected WebSocket evidence was not observed before timeout");
}
async function waitForReadyTerminal(page: any, timeoutMs: number): Promise<SurfaceState> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const state = await readSurfaceState(page);
      if (state.hasTerm && state.hasBuffer && state.hasTextarea && state.termCols && state.termRows) return state;
    } catch { /* retain the bounded wait */ }
    await pause(100);
  }
  fail("terminal", "ttyd did not expose the xterm terminal instance and input surface");
}
async function waitForTerminalText(page: any, needle: string, timeoutMs: number, stage: string): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try { if (await terminalHasText(page, needle)) return; } catch { /* retain the bounded wait */ }
    await pause(100);
  }
  fail(stage, "expected terminal output was not readable from the xterm buffer");
}
async function waitForTerminalSize(page: any, begin: string, end: string, timeoutMs: number): Promise<{ rows: number; cols: number }> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const size = await terminalSize(page, begin, end);
      if (size && size.rows > 0 && size.cols > 0) return size;
    } catch { /* retain the bounded wait */ }
    await pause(100);
  }
  fail("resize", "bounded stty size output was not readable from the xterm buffer");
}
async function typeCommand(page: any, command: string): Promise<void> {
  await browserStep("terminal", async () => {
    const input = page.locator(".xterm-helper-textarea");
    await input.focus(); await page.keyboard.insertText(command); await page.keyboard.press("Enter");
  });
}
const randomHex = () => randomBytes(6).toString("hex");

async function markerProbe(session: Session, timeoutMs: number): Promise<void> {
  const left = randomHex(); const right = randomHex(); const marker = `MWC_SHELL_MARKER_${left}_${right}`;
  const command = `printf '%s%s%s\\n' 'MWC_SHELL_' 'MARKER_${left}_' '${right}'`;
  if (command.includes(marker)) fail("marker", "marker was present in the typed command");
  await typeCommand(session.page, command); await waitForTerminalText(session.page, marker, timeoutMs, "marker");
  await waitForEvidence(session.evidence, (e) => e.requestCount > 0 && e.statuses.includes(101) && e.frameCount > 0, timeoutMs, "marker");
}
async function sizeProbe(session: Session, timeoutMs: number): Promise<{ rows: number; cols: number }> {
  const suffix = randomHex(); const begin = `MWC_SIZE_BEGIN_${suffix}_`; const end = `_MWC_SIZE_END_${suffix}`;
  await typeCommand(session.page, `printf '%s%s%s\\n' '${begin}' "$(stty size)" '${end}'`);
  return waitForTerminalSize(session.page, begin, end, timeoutMs);
}
async function waitForRuntimeResize(page: any, before: SurfaceState, timeoutMs: number): Promise<void> {
  if (before.termCols === null || before.termRows === null) fail("resize", "ttyd terminal instance did not expose dimensions");
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const current = await readSurfaceState(page);
      if (current.termCols !== before.termCols || current.termRows !== before.termRows) return;
    } catch { /* retain the bounded wait */ }
    await pause(100);
  }
  fail("resize", "xterm terminal dimensions did not change after viewport resize");
}
async function exerciseTerminal(session: Session, timeoutMs: number): Promise<void> {
  await waitForEvidence(session.evidence, (e) => e.requestCount > 0 && e.statuses.includes(101), timeoutMs, "websocket");
  const initial = await waitForReadyTerminal(session.page, timeoutMs);
  await markerProbe(session, timeoutMs);
  const beforeSize = await sizeProbe(session, timeoutMs);
  await browserStep("resize", () => session.page.setViewportSize({ width: 640, height: 420 }));
  await waitForRuntimeResize(session.page, initial, timeoutMs);
  const afterSize = await sizeProbe(session, timeoutMs);
  if (beforeSize.rows === afterSize.rows && beforeSize.cols === afterSize.cols) fail("resize", "stty size did not change after viewport resize");
  await waitForEvidence(session.evidence, (e) => e.requestCount > 0 && e.statuses.includes(101) && e.frameCount > 0, timeoutMs, "websocket");
}
async function verifyReplay(context: any, pages: Set<any>, ticket: BrowserTicket, timeoutMs: number): Promise<void> {
  const replay = await openTicketPage(context, pages, ticket, timeoutMs);
  try {
    await waitForEvidence(replay.evidence, (e) => e.requestCount > 0 && e.statuses.some((status) => status === 401 || status === 403), timeoutMs, "replay");
    if (replay.evidence.statuses.includes(101)) fail("replay", "replayed ticket unexpectedly opened a WebSocket");
  } finally { await replay.page.close().catch(() => undefined); pages.delete(replay.page); }
}

export async function runBrowserAcceptance(firstTicket: BrowserTicket, issueFreshTicket: () => Promise<BrowserTicket>, timeoutMs: number): Promise<void> {
  let browser: any; let context: any; const pages = new Set<any>();
  try {
    const playwright = await browserStep("browser", async () => {
      try { return await import("../web/node_modules/playwright-core/index.mjs"); } catch { fail("browser", "playwright-core could not be loaded"); }
    });
    browser = await browserStep("browser", async () => playwright.chromium.launch({
      executablePath: await findChromium(), headless: true, timeout: timeoutMs,
      args: ["--no-sandbox", "--disable-dev-shm-usage"],
    }));
    context = await browserStep("browser", () => browser.newContext({ viewport: { width: 1280, height: 760 } }));
    context.setDefaultTimeout(timeoutMs);
    const first = await openTicketPage(context, pages, firstTicket, timeoutMs);
    await exerciseTerminal(first, timeoutMs); await verifyReplay(context, pages, firstTicket, timeoutMs);
    await first.page.close().catch(() => undefined); pages.delete(first.page);
    const freshTicket = await issueFreshTicket();
    if (freshTicket.ticket === firstTicket.ticket) fail("ticket", "fresh ticket was not distinct from the consumed ticket");
    const fresh = await openTicketPage(context, pages, freshTicket, timeoutMs);
    try { await exerciseTerminal(fresh, timeoutMs); } finally {
      await fresh.page.close().catch(() => undefined); pages.delete(fresh.page);
    }
  } finally {
    for (const page of pages) await page.close().catch(() => undefined);
    if (context) await context.close().catch(() => undefined);
    if (browser) await browser.close().catch(() => undefined);
  }
}
