import { constants as fsConstants } from "node:fs";
import { access, lstat, open } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import {
  AcceptanceError,
  runBrowserAcceptance,
  type BrowserTicket,
} from "./web-shell-browser-acceptance-browser.ts";

const DEFAULT_TIMEOUT_MS = 30_000;
const MIN_TIMEOUT_MS = 5_000;
const MAX_TIMEOUT_MS = 120_000;
// UUID v7 is valid here; the API owns version/variant validation.
const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const USAGE = `Usage:
  node --experimental-strip-types scripts/web-shell-browser-acceptance.ts \\
    --console-url <http(s)://console> --workspace-id <ready-workspace-uuid> \\
    --token-file <owner-only-token-file> [--web-shell-origin <http(s)://origin>] \\
    [--timeout-ms <milliseconds>]

The runner keeps the token in memory and never writes screenshots or traces.
`;

type Options = {
  consoleUrl: URL; workspaceId: string; tokenFile: string;
  webShellOrigin: string | null; timeoutMs: number;
};
const fail = (stage: string, message: string): never => { throw new AcceptanceError(stage, message); };

function parseHttpUrl(value: string, label: string, originOnly = false): URL {
  let parsed: URL;
  try { parsed = new URL(value); } catch { fail("cli", `${label} must be a valid HTTP(S) URL`); }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") fail("cli", `${label} must use HTTP(S)`);
  if (parsed.username || parsed.password) fail("cli", `${label} must not contain URL credentials`);
  if (originOnly && (parsed.pathname !== "/" || parsed.search || parsed.hash)) fail("cli", `${label} must contain only an origin`);
  if (!originOnly && (parsed.search || parsed.hash)) fail("cli", "console URL must not contain a query or fragment");
  return parsed;
}

function parseArgs(argv: string[]): Options | null {
  const names = new Set(["--console-url", "--workspace-id", "--token-file", "--web-shell-origin", "--timeout-ms"]);
  const values: Record<string, string> = {};
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--help" || argument === "-h") return null;
    if (!names.has(argument)) fail("cli", "unknown argument; use --help for usage");
    if (values[argument] !== undefined) fail("cli", "an argument was provided more than once");
    const value = argv[index + 1];
    if (!value || value.startsWith("--")) fail("cli", `${argument} requires a value`);
    values[argument] = value; index += 1;
  }
  for (const name of ["--console-url", "--workspace-id", "--token-file"]) {
    if (!values[name]) fail("cli", `${name} is required`);
  }
  const workspaceId = values["--workspace-id"].toLowerCase();
  if (!UUID_PATTERN.test(workspaceId)) fail("cli", "workspace ID must be a UUID");
  let timeoutMs = DEFAULT_TIMEOUT_MS;
  if (values["--timeout-ms"] !== undefined) {
    timeoutMs = Number(values["--timeout-ms"]);
    if (!Number.isInteger(timeoutMs) || timeoutMs < MIN_TIMEOUT_MS || timeoutMs > MAX_TIMEOUT_MS) {
      fail("cli", `timeout must be between ${MIN_TIMEOUT_MS} and ${MAX_TIMEOUT_MS} milliseconds`);
    }
  }
  return {
    consoleUrl: parseHttpUrl(values["--console-url"], "console URL"), workspaceId,
    tokenFile: values["--token-file"],
    webShellOrigin: values["--web-shell-origin"] ? parseHttpUrl(values["--web-shell-origin"], "Web Shell origin", true).origin : null,
    timeoutMs,
  };
}

async function readToken(tokenFile: string): Promise<string> {
  let handle: Awaited<ReturnType<typeof open>> | undefined;
  try {
    if (fsConstants.O_NOFOLLOW === undefined) fail("token", "this platform cannot reject token file symlinks safely");
    if ((await lstat(tokenFile)).isSymbolicLink()) fail("token", "token file must not be a symbolic link");
    handle = await open(tokenFile, fsConstants.O_RDONLY | fsConstants.O_NOFOLLOW);
    const stat = await handle.stat();
    if (!stat.isFile()) fail("token", "token file must be a regular file");
    if (typeof process.getuid === "function" && stat.uid !== process.getuid()) fail("token", "token file must be owned by the current user");
    if ((stat.mode & 0o077) !== 0) fail("token", "token file permissions must not allow group or other access");
    if (stat.size > 16 * 1024) fail("token", "token file is unexpectedly large");
    const token = (await handle.readFile({ encoding: "utf8" })).trim();
    if (!token || /\s/.test(token)) fail("token", "token file must contain one non-empty token");
    return token;
  } catch (error) {
    if (error instanceof AcceptanceError) throw error;
    fail("token", "token file could not be opened safely");
  } finally { if (handle) await handle.close().catch(() => undefined); }
}

async function requestJson(url: URL, token: string, method: "GET" | "POST", expectedStatus: number, timeoutMs: number, stage: string): Promise<any> {
  const controller = new AbortController(); let timedOut = false;
  const timer = setTimeout(() => { timedOut = true; controller.abort(); }, timeoutMs);
  try {
    const response = await fetch(url, {
      method, headers: { Accept: "application/json", Authorization: `Bearer ${token}` },
      redirect: "error", signal: controller.signal,
    });
    if (response.status !== expectedStatus) fail(stage, `API returned HTTP ${response.status}`);
    try { return await response.json(); } catch { fail(stage, "API returned invalid JSON"); }
  } catch (error) {
    if (error instanceof AcceptanceError) throw error;
    fail(stage, timedOut ? "API request timed out" : "API request failed");
  } finally { clearTimeout(timer); }
}
function workspaceApiUrl(options: Options, suffix = ""): URL {
  return new URL(`/api/v1/workspaces/${encodeURIComponent(options.workspaceId)}${suffix}`, options.consoleUrl);
}
async function verifyIdentityAndReady(options: Options, token: string): Promise<void> {
  const principal = await requestJson(new URL("/api/v1/me", options.consoleUrl), token, "GET", 200, options.timeoutMs, "identity");
  const userId = typeof principal?.user_id === "string" ? principal.user_id.toLowerCase() : "";
  if (!UUID_PATTERN.test(userId)) fail("identity", "authenticated principal identity was invalid");
  const payload = await requestJson(workspaceApiUrl(options), token, "GET", 200, options.timeoutMs, "workspace");
  const workspace = payload && typeof payload === "object" ? payload.workspace : null;
  if (!workspace || typeof workspace.id !== "string" || workspace.id.toLowerCase() !== options.workspaceId) {
    fail("workspace", "workspace response did not match the requested workspace");
  }
  if (typeof workspace.owner_id !== "string" || workspace.owner_id.toLowerCase() !== userId) {
    fail("workspace", "workspace is not owned by the authenticated principal");
  }
  if (workspace.state !== "ready") fail("workspace", "workspace is not Ready");
}

function validateTicket(payload: any, options: Options): BrowserTicket {
  if (!payload || typeof payload !== "object") fail("ticket", "ticket API returned an invalid response");
  const ticket = payload.ticket;
  if (typeof ticket !== "string" || !ticket || /\s/.test(ticket)) fail("ticket", "ticket API returned an invalid ticket");
  if (payload.workspace_id !== options.workspaceId) fail("ticket", "ticket API returned the wrong workspace");
  if (typeof payload.expires_at !== "number" || !Number.isFinite(payload.expires_at)) fail("ticket", "ticket API returned an invalid expiry");
  if (payload.expires_at <= Math.floor(Date.now() / 1000)) fail("ticket", "ticket API returned an expired ticket");
  if (typeof payload.web_shell_url !== "string" || !payload.web_shell_url) fail("ticket", "ticket API returned an invalid Web Shell address");
  let url: URL;
  try { url = new URL(payload.web_shell_url, options.consoleUrl); } catch { fail("ticket", "ticket API returned an invalid Web Shell address"); }
  if (url.protocol !== "http:" && url.protocol !== "https:") fail("ticket", "Web Shell address must use HTTP(S)");
  if (url.username || url.password || url.hash || !url.pathname.startsWith("/shell/")) fail("ticket", "Web Shell address is not an allowed ttyd route");
  if (url.origin !== (options.webShellOrigin ?? options.consoleUrl.origin)) fail("ticket", "Web Shell address origin is not allowed");
  if (url.searchParams.get("ticket") !== ticket) fail("ticket", "Web Shell address does not carry the issued ticket");
  return { ticket, url };
}
async function issueTicket(options: Options, token: string): Promise<BrowserTicket> {
  const payload = await requestJson(workspaceApiUrl(options, "/web-shell-tickets"), token, "POST", 201, options.timeoutMs, "ticket");
  return validateTicket(payload, options);
}

async function runAcceptance(options: Options): Promise<void> {
  let token = "";
  try {
    token = await readToken(options.tokenFile);
    await verifyIdentityAndReady(options, token);
    const firstTicket = await issueTicket(options, token);
    await runBrowserAcceptance(firstTicket, () => issueTicket(options, token), options.timeoutMs);
  } finally { token = ""; }
}

async function main(): Promise<void> {
  try {
    const options = parseArgs(process.argv.slice(2));
    if (!options) { console.log(USAGE); return; }
    await runAcceptance(options);
    console.log("Web Shell browser acceptance passed: Ready workspace, ttyd WebSocket, command output, resize, replay rejection, and fresh-ticket recovery verified.");
  } catch (error) {
    if (error instanceof AcceptanceError) console.error(`Web Shell browser acceptance failed (${error.stage}): ${error.message}`);
    else console.error("Web Shell browser acceptance failed (runner): unexpected runner failure");
    process.exitCode = 1;
  }
}

export { parseArgs, runAcceptance };
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) void main();
