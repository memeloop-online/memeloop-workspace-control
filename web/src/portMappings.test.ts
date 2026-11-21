import assert from "node:assert/strict";
import test from "node:test";
import { mappingUrl, parseInternalPort, safeBootstrapUrl, validateSubdomain } from "./portMappings.ts";
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
  assert.equal(mappingUrl({ id: "a", internal_port: 3000, display_name: null, subdomain: "app-1a2b3c4d", status: "ready", https_url: "https://x", managed: false, purpose: "user" }), "https://x");
  assert.equal(mappingUrl({ id: "a", internal_port: 3000, display_name: null, subdomain: "app-1a2b3c4d", status: "provisioning", https_url: null, managed: false, purpose: "user" }), null);
});

test("does not turn a non-HTTPS field into a browser destination", () => {
  assert.equal(
    mappingUrl({ id: "a", internal_port: 3000, display_name: null, subdomain: "app-1a2b3c4d", status: "ready", https_url: null, managed: false, purpose: "user", url: "http://x" } as PortMapping),
    null,
  );
});

test("accepts only non-credentialed HTTPS bootstrap destinations", () => {
  assert.equal(safeBootstrapUrl("https://desktop.example.test/session"), "https://desktop.example.test/session");
  assert.equal(safeBootstrapUrl("http://desktop.example.test/session"), null);
  assert.equal(safeBootstrapUrl("javascript:alert(1)"), null);
  assert.equal(safeBootstrapUrl("https://token@desktop.example.test/session"), null);
  assert.equal(safeBootstrapUrl("/relative-path"), null);
});

test("display names count trimmed Chinese and emoji characters rather than bytes or UTF-16 units", async () => {
  const { validateDisplayName } = await import("./portMappings.ts");
  for (const character of ["名", "😀"]) {
    assert.deepEqual(validateDisplayName(`  ${character.repeat(80)}  `), { value: character.repeat(80), length: 80, tooLong: false });
    assert.equal(validateDisplayName(character.repeat(81)).tooLong, true);
  }
  assert.deepEqual(validateDisplayName("   "), { value: "", length: 0, tooLong: false });
  for (const port of ["22", "2222", "3389", "7681", "8080", "8081", "8443"]) assert.equal(parseInternalPort(port), null);
});

test("normalizes and validates optional subdomain labels as DNS single labels", () => {
  assert.deepEqual(validateSubdomain("  My-App  "), { value: "my-app", valid: true });
  assert.deepEqual(validateSubdomain("   "), { value: "", valid: true });
  assert.equal(validateSubdomain("a").valid, true);
  assert.equal(validateSubdomain("app-123").valid, true);
  assert.equal(validateSubdomain("a".repeat(63)).valid, true);
  assert.equal(validateSubdomain("a".repeat(64)).valid, false);
  assert.equal(validateSubdomain("-app").valid, false);
  assert.equal(validateSubdomain("app-").valid, false);
  assert.equal(validateSubdomain("app_dev").valid, false);
  assert.equal(validateSubdomain("app.dev").valid, false);
  assert.equal(validateSubdomain("应用").valid, false);
  assert.equal(validateSubdomain("APP-123").value, "app-123");
});
