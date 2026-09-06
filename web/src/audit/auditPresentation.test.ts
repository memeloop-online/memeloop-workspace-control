import assert from "node:assert/strict";
import test from "node:test";
import { auditActionLabel, hasKnownAuditAction } from "./auditPresentation.ts";

test("current mutable resources have product action labels", () => {
  for (const action of [
    "organization.update",
    "organization.delete",
    "user.update",
    "workspace.port_mapping.create",
    "workspace.port_mapping.delete",
  ]) {
    assert.equal(hasKnownAuditAction(action), true);
    assert.notEqual(auditActionLabel(action), "auditOtherAction");
  }
});

test("future actions remain diagnosable without implying an authentication failure", () => {
  assert.equal(hasKnownAuditAction("extension.example"), false);
  assert.equal(auditActionLabel("extension.example"), "auditOtherAction");
});
