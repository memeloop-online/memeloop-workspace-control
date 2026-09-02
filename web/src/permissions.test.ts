import assert from "node:assert/strict";
import test from "node:test";

import { canManageOrganization, canManageSystem } from "./permissions.ts";
import type { Principal } from "./types.ts";

const principal: Principal = {
  user_id: "user-1",
  display_name: "Admin",
  system_admin: true,
  memberships: [],
  api_key_scopes: ["manage_system"],
};

test("system status cannot bypass the authenticating key scope", () => {
  assert.equal(canManageSystem(principal), true);
  assert.equal(canManageOrganization(principal, "org-1", "manage_members"), false);
});

test("organization administration requires both role and key scope", () => {
  const organizationAdmin: Principal = {
    ...principal,
    system_admin: false,
    memberships: [{ organization_id: "org-1", role: "organization_admin" }],
    api_key_scopes: ["manage_members"],
  };
  assert.equal(canManageOrganization(organizationAdmin, "org-1", "manage_members"), true);
  assert.equal(canManageOrganization(organizationAdmin, "org-2", "manage_members"), false);
  assert.equal(canManageOrganization(organizationAdmin, "org-1", "manage_organization"), false);
});
