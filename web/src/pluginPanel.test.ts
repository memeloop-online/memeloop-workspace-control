import assert from "node:assert/strict";
import test from "node:test";

import { checkPluginSchema, configurationKey } from "./plugins/schema.ts";
import { nextConfigurationScope, pluginCatalogState, pluginErrorMessageKey } from "./plugins/viewModel.ts";

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
