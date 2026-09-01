import assert from "node:assert/strict";
import test from "node:test";
import { mappingUrl, parseInternalPort } from "./portMappings.ts";
import type { PortMapping } from "./portMappings.ts";

test("validates internal ports without accepting decimals or reserved values", () => {
  assert.equal(parseInternalPort("1"), null);
  assert.equal(parseInternalPort("80"), 80);
  assert.equal(parseInternalPort("443"), 443);
  assert.equal(parseInternalPort("2222"), null);
  assert.equal(parseInternalPort("7681"), null);
  assert.equal(parseInternalPort("65535"), 65535);
  assert.equal(parseInternalPort("0"), null);
  assert.equal(parseInternalPort("65536"), null);
  assert.equal(parseInternalPort("3000.5"), null);
  assert.equal(parseInternalPort(""), null);
});

test("prefers HTTPS mapping URLs", () => {
  assert.equal(mappingUrl({ id: "a", internal_port: 3000, display_name: null, status: "ready", https_url: "https://x" }), "https://x");
  assert.equal(mappingUrl({ id: "a", internal_port: 3000, display_name: null, status: "provisioning", https_url: null }), null);
});

test("does not turn a legacy or non-HTTPS field into a browser destination", () => {
  assert.equal(
    mappingUrl({ id: "a", internal_port: 3000, display_name: null, status: "ready", https_url: null, url: "http://x" } as PortMapping),
    null,
  );
});
