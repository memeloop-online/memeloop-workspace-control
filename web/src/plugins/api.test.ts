import assert from "node:assert/strict";
import test from "node:test";

import { PluginApi } from "./api.ts";
import type { PluginInspection } from "./types.ts";

const inspection: PluginInspection = {
  inspection_id: "00000000-0000-0000-0000-000000000001",
  expires_at: 2_000_000_000,
  digest: "a".repeat(64),
  size_bytes: 1024,
  source_kind: "url",
  source_ref: "https://plugins.example/plugin.mwcpkg",
  manifest: { id: "example.plugin", name: "Example", version: "1.0.0", description: "Example plugin", wit_version: "0.1.0" },
  declared_contributions: ["configuration", "ui_surfaces"],
  current_package_version: 0,
};

test("remote inspection uses the fixed endpoint, digest, and idempotency header", async () => {
  const original = globalThis.fetch;
  let captured: { input: RequestInfo | URL; init?: RequestInit } | undefined;
  globalThis.fetch = async (input, init) => {
    captured = { input, init };
    return Response.json(inspection);
  };
  try {
    await new PluginApi("token").inspectUrl("https://plugins.example/plugin.mwcpkg", "a".repeat(64));
    assert.equal(captured?.input, "/api/v1/plugins/inspections/url");
    assert.deepEqual(JSON.parse(String(captured?.init?.body)), { url: "https://plugins.example/plugin.mwcpkg", expected_sha256: "a".repeat(64) });
    const headers = captured?.init?.headers as Headers;
    assert.equal(headers.get("Authorization"), "Bearer token");
    assert.ok(headers.get("Idempotency-Key"));
  } finally { globalThis.fetch = original; }
});

test("local inspection sends separate manifest, component, and repeated assets", async () => {
  const original = globalThis.fetch;
  let body: FormData | undefined;
  let headers: Headers | undefined;
  globalThis.fetch = async (_input, init) => {
    body = init?.body as FormData;
    headers = init?.headers as Headers;
    return Response.json({ ...inspection, source_kind: "file", source_ref: "manifest.json" });
  };
  try {
    const manifest = new File(["{}"], "manifest.json", { type: "application/json" });
    const component = new File(["component"], "plugin.component");
    const assets = [new File(["page"], "index.html", { type: "text/html" }), new File(["style"], "style.css", { type: "text/css" })];
    await new PluginApi("token").inspectLocalPackage(manifest, component, assets);
    assert.equal((body?.get("manifest") as File).name, "manifest.json");
    assert.equal((body?.get("component") as File).name, "plugin.component");
    assert.deepEqual(body?.getAll("asset").map((value) => (value as File).name), ["index.html", "style.css"]);
    assert.equal(headers?.has("Content-Type"), false);
  } finally { globalThis.fetch = original; }
});

test("uninstall accepts the backend's empty 204 response", async () => {
  const original = globalThis.fetch;
  globalThis.fetch = async () => new Response(null, { status: 204 });
  try {
    assert.equal(await new PluginApi("token").uninstall("example.plugin", 3), undefined);
  } finally { globalThis.fetch = original; }
});

test("installation forwards every explicitly approved declared contribution", async () => {
  const original = globalThis.fetch;
  let requestBody: Record<string, unknown> | undefined;
  globalThis.fetch = async (_input, init) => {
    requestBody = JSON.parse(String(init?.body)) as Record<string, unknown>;
    return Response.json({ id: "example.plugin" });
  };
  try {
    const contributions = ["workspace_create_policy", "configuration", "ui_surfaces", "api_routes", "api_middleware"] as const;
    await new PluginApi("token").install({ ...inspection, declared_contributions: [...contributions] }, [...contributions], true);
    assert.deepEqual(requestBody?.approved_contributions, contributions);
    assert.equal(requestBody?.expected_package_version, 0);
    assert.equal(requestBody?.enabled, true);
  } finally { globalThis.fetch = original; }
});

test("plugin page API calls only its own declared route with the host identity", async () => {
  const original = globalThis.fetch;
  let captured: { input: RequestInfo | URL; init?: RequestInit } | undefined;
  globalThis.fetch = async (input, init) => {
    captured = { input, init };
    return new Response(JSON.stringify({ ok: true }), { status: 201, headers: { "Content-Type": "application/json", "Content-Length": "11" } });
  };
  try {
    const result = await new PluginApi("host-token").invokePluginRoute("example.plugin", { route_id: "summary", method: "POST", path: "reports/today", body: { page: 1 } }, "00000000-0000-0000-0000-000000000002");
    assert.equal(captured?.input, "/api/v1/plugin-api/example.plugin/summary/reports/today?organization_id=00000000-0000-0000-0000-000000000002");
    const headers = captured?.init?.headers as Headers;
    assert.equal(headers.get("Authorization"), "Bearer host-token");
    assert.equal(headers.get("Content-Type"), "application/json");
    assert.equal(captured?.init?.credentials, "same-origin");
    assert.deepEqual(result, { status: 201, content_type: "application/json", body: JSON.stringify({ ok: true }) });
  } finally { globalThis.fetch = original; }
});

test("plugin page API rejects oversized or non-text responses", async () => {
  const original = globalThis.fetch;
  try {
    globalThis.fetch = async () => new Response("large", { headers: { "Content-Type": "text/plain", "Content-Length": String(1024 * 1024 + 1) } });
    await assert.rejects(() => new PluginApi("token").invokePluginRoute("example.plugin", { route_id: "summary", method: "GET", path: "" }, null), /plugin_api_response_too_large/u);
    globalThis.fetch = async () => new Response(new Uint8Array([1, 2, 3]), { headers: { "Content-Type": "application/octet-stream" } });
    await assert.rejects(() => new PluginApi("token").invokePluginRoute("example.plugin", { route_id: "summary", method: "GET", path: "" }, null), /plugin_api_response_type_invalid/u);
  } finally { globalThis.fetch = original; }
});
