import assert from "node:assert/strict";
import test from "node:test";
import { auditActionLabel, auditStateLabel, hasKnownAuditAction } from "./auditPresentation.ts";
import type { AuditRecord } from "../types.ts";

function auditRecord(action: string): AuditRecord {
  return {
    id: "audit-1",
    actor_user_id: null,
    actor_display_name: null,
    organization_id: null,
    workspace_id: "workspace-1",
    workspace_name: "fixture",
    workspace_short_id: "fixture",
    action,
    metadata: {},
    created_at: 1_700_000_000,
  };
}

test("current mutable resources have product action labels", () => {
  for (const action of [
    "organization.update",
    "organization.delete",
    "user.update",
    "user.api_key.token_read",
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

test("a workspace losing its serving pod reads as returning to Starting, not a user action", () => {
  assert.equal(hasKnownAuditAction("workspace.mark_not_ready"), true);
  assert.notEqual(auditActionLabel("workspace.mark_not_ready"), "auditOtherAction");
  assert.equal(auditActionLabel("workspace.mark_not_ready"), "stateStarting");
  assert.equal(auditStateLabel(auditRecord("workspace.mark_not_ready")), "stateStarting");
});
