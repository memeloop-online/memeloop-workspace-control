import { pathToFileURL } from "node:url";

const DEFAULT_TIMEOUT_MS = 15_000;
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

type Options = { baseUrl: URL; tokenEnv: string; timeoutMs: number };
type CreatedKey = { id: string; token: string };

class AcceptanceError extends Error {}

const usage = `Usage:
  MWC_LIVE_ADMIN_TOKEN=<token-in-memory> node --experimental-strip-types \\
    scripts/live-key-boundary-acceptance.ts --base-url <https://workspace.example>

Optional: --token-env <environment-variable-name> --timeout-ms <5000..60000>
The token is read only from process memory and is never logged or written.\n`;

function fail(message: string): never { throw new AcceptanceError(message); }

function optionsFrom(argv: string[]): Options | null {
  const values: Record<string, string> = {};
  const names = new Set(["--base-url", "--token-env", "--timeout-ms"]);
  for (let index = 0; index < argv.length; index += 1) {
    const name = argv[index];
    if (name === "--help" || name === "-h") return null;
    if (!names.has(name) || values[name] !== undefined) fail("invalid command-line arguments");
    const value = argv[index + 1];
    if (!value || value.startsWith("--")) fail(`missing value for ${name}`);
    values[name] = value;
    index += 1;
  }
  if (!values["--base-url"]) fail("--base-url is required");
  let baseUrl: URL;
  try { baseUrl = new URL(values["--base-url"]); } catch { fail("--base-url must be a valid HTTP(S) origin"); }
  if (!["http:", "https:"].includes(baseUrl.protocol) || baseUrl.username || baseUrl.password
    || baseUrl.pathname !== "/" || baseUrl.search || baseUrl.hash) fail("--base-url must be a valid HTTP(S) origin");
  const tokenEnv = values["--token-env"] ?? "MWC_LIVE_ADMIN_TOKEN";
  if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(tokenEnv)) fail("--token-env is invalid");
  const timeoutMs = values["--timeout-ms"] === undefined ? DEFAULT_TIMEOUT_MS : Number(values["--timeout-ms"]);
  if (!Number.isInteger(timeoutMs) || timeoutMs < 5_000 || timeoutMs > 60_000) fail("--timeout-ms must be an integer from 5000 to 60000");
  return { baseUrl, tokenEnv, timeoutMs };
}

function tokenFromEnvironment(name: string): string {
  const token = process.env[name]?.trim() ?? "";
  if (!token || /\s/.test(token)) fail("the configured token environment variable is empty or invalid");
  return token;
}

function apiUrl(options: Options, path: string): URL { return new URL(path, options.baseUrl); }

async function request(options: Options, token: string, method: string, path: string, body?: unknown): Promise<{ status: number; json: unknown }> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), options.timeoutMs);
  try {
    const response = await fetch(apiUrl(options, path), {
      method,
      redirect: "error",
      signal: controller.signal,
      headers: { Accept: "application/json", Authorization: `Bearer ${token}`, ...(body === undefined ? {} : { "Content-Type": "application/json" }) },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    let json: unknown = null;
    const text = await response.text();
    if (text) { try { json = JSON.parse(text); } catch { /* status-only checks intentionally ignore malformed error bodies */ } }
    return { status: response.status, json };
  } catch {
    fail(`request failed for ${method} ${path}`);
  } finally { clearTimeout(timer); }
}

function expectStatus(actual: number, expected: number, label: string): void {
  if (actual !== expected) fail(`${label}: expected HTTP ${expected}, got HTTP ${actual}`);
}

function keyFrom(response: { status: number; json: unknown }, label: string): CreatedKey {
  expectStatus(response.status, 201, label);
  const value = response.json as Record<string, unknown> | null;
  const id = typeof value?.id === "string" ? value.id : "";
  const token = typeof value?.token === "string" ? value.token : "";
  if (!UUID.test(id) || !token || /\s/.test(token)) fail(`${label}: API returned an invalid temporary key response`);
  return { id, token };
}

function templateIdFrom(value: unknown): string {
  if (!Array.isArray(value)) fail("template listing had an unexpected response");
  const template = value.find((item): item is Record<string, unknown> => typeof item === "object" && item !== null && UUID.test(String((item as Record<string, unknown>).id ?? "")));
  if (!template || typeof template.id !== "string") fail("no existing template is available; this acceptance intentionally will not create one");
  return template.id;
}

async function revoke(options: Options, administratorToken: string, key: CreatedKey): Promise<void> {
  const result = await request(options, administratorToken, "DELETE", `/api/v1/me/api-keys/${encodeURIComponent(key.id)}`);
  expectStatus(result.status, 204, "temporary key revocation");
  const revoked = await request(options, key.token, "GET", "/api/v1/me");
  expectStatus(revoked.status, 401, "revoked key authentication");
}

async function expectRejectedMint(options: Options, parent: CreatedKey, body: object, label: string, created: CreatedKey[]): Promise<void> {
  const result = await request(options, parent.token, "POST", "/api/v1/me/api-keys", body);
  if (result.status === 201) {
    // A regression must not leave a usable key behind. Register it before failing so finally revokes it.
    created.push(keyFrom(result, `${label} unexpected creation`));
    fail(`${label}: unsafe child key was created unexpectedly`);
  }
  expectStatus(result.status, 403, label);
}

async function run(options: Options): Promise<void> {
  const administratorToken = tokenFromEnvironment(options.tokenEnv);
  const created: CreatedKey[] = [];
  let primaryFailure: unknown;
  try {
    expectStatus((await request(options, administratorToken, "GET", "/api/v1/me")).status, 200, "administrator identity");
    const templates = await request(options, administratorToken, "GET", "/api/v1/templates");
    expectStatus(templates.status, 200, "template listing");
    const allowedTemplateId = templateIdFrom(templates.json);
    const now = Math.floor(Date.now() / 1000);
    const parentExpiry = now + 600;
    const parent = keyFrom(await request(options, administratorToken, "POST", "/api/v1/me/api-keys", {
      name: `live-boundary-parent-${now}`,
      scopes: ["manage_api_keys", "read_workspace"],
      expires_at: parentExpiry,
      allowed_template_ids: [allowedTemplateId],
    }), "restricted parent key creation");
    created.push(parent);

    expectStatus((await request(options, parent.token, "GET", "/api/v1/me")).status, 200, "restricted parent identity");
    await expectRejectedMint(options, parent, {
      name: "live-boundary-unrestricted-child", scopes: ["read_workspace"], expires_at: parentExpiry - 60, allowed_template_ids: null,
    }, "unrestricted child rejection", created);
    await expectRejectedMint(options, parent, {
      name: "live-boundary-longer-child", scopes: ["read_workspace"], expires_at: parentExpiry + 60, allowed_template_ids: [allowedTemplateId],
    }, "longer-lived child rejection", created);

    const child = keyFrom(await request(options, parent.token, "POST", "/api/v1/me/api-keys", {
      name: `live-boundary-child-${now}`,
      scopes: ["read_workspace"],
      expires_at: parentExpiry - 60,
      allowed_template_ids: [allowedTemplateId],
    }), "bounded child key creation");
    created.push(child);
    expectStatus((await request(options, child.token, "GET", "/api/v1/me")).status, 200, "bounded child identity");
    expectStatus((await request(options, child.token, "GET", "/api/v1/templates")).status, 200, "bounded child read");
    expectStatus((await request(options, child.token, "GET", "/api/v1/me/api-keys")).status, 403, "scope-exceeded read rejection");
  } catch (error) {
    primaryFailure = error;
  } finally {
    const cleanupFailures: string[] = [];
    for (const key of created.reverse()) {
      try { await revoke(options, administratorToken, key); } catch { cleanupFailures.push("temporary-key cleanup or post-revocation verification failed"); }
    }
    if (cleanupFailures.length) fail(cleanupFailures[0]);
  }
  if (primaryFailure) throw primaryFailure;
}

async function main(): Promise<void> {
  try {
    const options = optionsFrom(process.argv.slice(2));
    if (!options) { console.log(usage); return; }
    await run(options);
    console.log("PASS live API-key boundary acceptance: temporary restricted key, child read, privilege rejection, unrestricted-child rejection, longer-expiry rejection, and revocation-to-401.");
  } catch (error) {
    console.error(`FAIL live API-key boundary acceptance: ${error instanceof Error ? error.message : "unexpected failure"}`);
    process.exitCode = 1;
  }
}

export { optionsFrom, run };
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) void main();
