import assert from "node:assert/strict";
import test from "node:test";

import { checkPluginSchema, configurationKey } from "./plugins/schema.ts";
import { nextConfigurationScope, pluginCatalogState, pluginErrorMessageKey, isGithubRepository, isSha256, pluginSourceSummary } from "./plugins/viewModel.ts";
import { currentPluginTheme, parsePluginApiBridgeRequest, parsePluginBridgeRequest, safePluginApiRelativePath, safePluginSessionPath } from "./plugins/surfaceBridge.ts";

test("plugin catalog distinguishes loading, failure, empty, and ready states", () => {
  assert.equal(pluginCatalogState(true, "", 0), "loading");
  assert.equal(pluginCatalogState(false, "offline", 0), "error");
  assert.equal(pluginCatalogState(false, "", 0), "empty");
  assert.equal(pluginCatalogState(false, "", 1), "ready");
});

test("plugin configuration scope tabs support keyboard navigation", () => {
  assert.equal(nextConfigurationScope("organization", "ArrowLeft", true), "installation");
  assert.equal(nextConfigurationScope("installation", "ArrowRight", true), "organization");
  assert.equal(nextConfigurationScope("organization", "Home", true), "installation");
  assert.equal(nextConfigurationScope("installation", "End", true), "organization");
  assert.equal(nextConfigurationScope("organization", "ArrowLeft", false), "organization");
});

test("plugin API errors map to stable localized UI states", () => {
  assert.equal(pluginErrorMessageKey("plugin_configuration_version_conflict"), "pluginVersionConflict");
  assert.equal(pluginErrorMessageKey("invalid_plugin_configuration"), "pluginInvalidConfiguration");
  assert.equal(pluginErrorMessageKey("plugin_not_found"), "pluginNotFound");
  assert.equal(pluginErrorMessageKey("unexpected"), "pluginRequestFailed");
});

test("plugin schemas reject credentials, external references, and unsafe patterns", () => {
  assert.deepEqual(checkPluginSchema({ type: "object", properties: { api_token: { type: "string" } } }), { ok: false, reason: "sensitive" });
  assert.deepEqual(checkPluginSchema({ type: "object", properties: { mode: { $ref: "https://example.test/schema" } } }), { ok: false, reason: "unsupported" });
  assert.deepEqual(checkPluginSchema({ type: "object", properties: { value: { type: "string", pattern: "(a+)+$" } } }), { ok: false, reason: "unsupported" });
  assert.deepEqual(checkPluginSchema({ type: "object", description: "<a href='https://example.test'>click</a>", properties: {} }), { ok: false, reason: "unsupported" });
});

test("safe declarative plugin schemas and installation keys are accepted", () => {
  const schema = { type: "object", required: ["mode"], properties: { mode: { type: "string", enum: ["audit", "enforce"] } }, additionalProperties: false };
  const checked = checkPluginSchema(schema);
  assert.equal(checked.ok, true);
  assert.equal(configurationKey("policy.example", "installation"), "policy.example:installation");
});

test("remote plugin sources require complete repository and digest values", () => {
  assert.equal(isGithubRepository("memeloop-online/example"), true);
  assert.equal(isGithubRepository("memeloop-online"), false);
  assert.equal(isSha256("a".repeat(64)), true);
  assert.equal(isSha256("a".repeat(63)), false);
  assert.equal(pluginSourceSummary("github_release", "ignored", { kind: "github_release", repository: "memeloop-online/example", tag: "v1", asset: "example.mwc-plugin" }), "memeloop-online/example · v1 · example.mwc-plugin");
});

test("plugin page sessions remain same-origin and under their fixed API paths", () => {
  assert.equal(safePluginSessionPath("/api/v1/plugin-ui/example/session/index.html?ticket=one", "https://workspace.example"), "/api/v1/plugin-ui/example/session/index.html?ticket=one");
  assert.equal(safePluginSessionPath("https://attacker.example/plugin", "https://workspace.example"), null);
});

test("plugin page bridge rejects an invalid nonce or an ungranted method", () => {
  const valid = { type: "mwc:bridge", nonce: "nonce", request_id: "request-1", method: "plugin_api.request", payload: { route_id: "summary", method: "GET", path: "today" } };
  assert.deepEqual(parsePluginBridgeRequest(valid, "nonce", ["plugin_api.request"]), { request_id: "request-1", method: "plugin_api.request", payload: valid.payload });
  assert.equal(parsePluginBridgeRequest(valid, "different", ["plugin_api.request"]), null);
  assert.equal(parsePluginBridgeRequest({ ...valid, method: "workspace.read" }, "nonce", ["workspace.read"]), null);
  assert.equal(parsePluginBridgeRequest({ ...valid, method: "clipboard.write" }, "nonce", ["clipboard.write"]), null);
});

test("plugin API bridge accepts only declared routes, bounded JSON, and safe relative paths", () => {
  assert.deepEqual(
    parsePluginApiBridgeRequest({ route_id: "summary", method: "POST", path: "/reports/今日", body: { page: 1 } }, ["summary"]),
    { route_id: "summary", method: "POST", path: "reports/%E4%BB%8A%E6%97%A5", body: { page: 1 } },
  );
  assert.equal(parsePluginApiBridgeRequest({ route_id: "other", method: "GET", path: "" }, ["summary"]), null);
  assert.equal(parsePluginApiBridgeRequest({ route_id: "summary", method: "GET", path: "", headers: { Authorization: "stolen" } }, ["summary"]), null);
  assert.equal(parsePluginApiBridgeRequest({ route_id: "summary", method: "GET", path: "", body: {} }, ["summary"]), null);
  assert.equal(parsePluginApiBridgeRequest({ route_id: "summary", method: "OPTIONS", path: "" }, ["summary"]), null);
  assert.equal(parsePluginApiBridgeRequest({ route_id: "summary", method: "POST", path: "safe", body: "x".repeat(256 * 1024 + 1) }, ["summary"]), null);
  assert.equal(safePluginApiRelativePath("../audit"), null);
  assert.equal(safePluginApiRelativePath("route?organization_id=other"), null);
});

test("plugin theme bridge reads the active application theme", () => {
  assert.equal(currentPluginTheme({ dataset: { theme: "light" } }), "light");
  assert.equal(currentPluginTheme({ dataset: { theme: "dark" } }), "dark");
});
