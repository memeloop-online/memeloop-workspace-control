import assert from "node:assert/strict";
import test from "node:test";

import { canManageOrganization, canManageOtherUserApiKeys, canManageSystem, canShrinkWorkspaceStorage } from "./permissions.ts";
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

test("other-user API-key configuration requires an unrestricted system administrator key", () => {
  const administrator = { ...principal, api_key_scopes: ["manage_system", "manage_api_keys"] as Principal["api_key_scopes"], allowed_template_ids: null };
  assert.equal(canManageOtherUserApiKeys(administrator), true);
  assert.equal(canManageOtherUserApiKeys({ ...administrator, system_admin: false }), false);
  assert.equal(canManageOtherUserApiKeys({ ...administrator, api_key_scopes: ["manage_api_keys"] }), false);
  assert.equal(canManageOtherUserApiKeys({ ...administrator, api_key_scopes: ["manage_system"] }), false);
  assert.equal(canManageOtherUserApiKeys({ ...administrator, allowed_template_ids: [] }), false);
});

test("storage shrink follows the workspace management policy, not system-admin only", () => {
  const workspace = { owner_id: "user-1", organization_id: "org-1" };
  const owner: Principal = { ...principal, system_admin: false, api_key_scopes: ["change_workspace_state"] };
  assert.equal(canShrinkWorkspaceStorage(owner, workspace), true);
  const organizationAdmin: Principal = { ...owner, user_id: "user-2", memberships: [{ organization_id: "org-1", role: "organization_admin" }] };
  assert.equal(canShrinkWorkspaceStorage(organizationAdmin, workspace), true);
  const systemAdmin: Principal = { ...owner, user_id: "user-3", system_admin: true };
  assert.equal(canShrinkWorkspaceStorage(systemAdmin, workspace), true);
  const otherMember: Principal = { ...owner, user_id: "user-4", memberships: [{ organization_id: "org-1", role: "member" }] };
  assert.equal(canShrinkWorkspaceStorage(otherMember, workspace), false);
  const ownerWithoutScope: Principal = { ...owner, api_key_scopes: [] };
  assert.equal(canShrinkWorkspaceStorage(ownerWithoutScope, workspace), false);
});
