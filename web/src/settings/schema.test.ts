import assert from "node:assert/strict";
import test from "node:test";

import type { MessageKey } from "../i18n.tsx";
import type { Principal } from "../types.ts";
import { matchingFields } from "./fields.ts";
import { visibleSettingsSections } from "./schema.ts";

const translation = (key: MessageKey) => ({ avatarUrl: "头像", displayName: "显示名称", userApiCredentials: "凭据配置", cpu: "处理器", memory: "内存", copy: "复制" } as Partial<Record<MessageKey, string>>)[key] ?? key;
const member: Principal = {
  user_id: "member-1",
  display_name: "Member",
  system_admin: false,
  memberships: [{ organization_id: "org-1", role: "member" }],
  api_key_scopes: ["manage_api_keys"],
  api_key_expires_at: null,
  allowed_template_ids: null,
};

test("permission filtering runs before category and search", () => {
  assert.deepEqual(visibleSettingsSections(member, "org-1", "system", "", translation), []);
  assert.deepEqual(visibleSettingsSections(member, "org-1", "all", "image", translation), []);
  assert.deepEqual(visibleSettingsSections(member, "org-1", "organization", "createOrganization", translation), []);
  assert.ok(visibleSettingsSections(member, "", "personal", "api key", translation).some((section) => section.id === "api-keys"));
});

test("simple fields index stable keys and translated labels", () => {
  assert.deepEqual(matchingFields("profile", "头像", translation).map((field) => field.key), ["avatar_url"]);
  assert.deepEqual(matchingFields("profile", "display_name", translation).map((field) => field.key), ["display_name"]);
  assert.deepEqual(visibleSettingsSections(member, "org-1", "personal", "头像", translation).map((section) => section.id), ["profile"]);
  assert.deepEqual(visibleSettingsSections(member, "org-1", "organization", "凭据配置", translation).map((section) => section.id), []);
});

test("administrative sections filter independently", () => {
  const administrator: Principal = { ...member, system_admin: true, api_key_scopes: ["manage_system", "manage_api_keys", "manage_organization", "manage_members"], allowed_template_ids: null };
  assert.deepEqual(visibleSettingsSections(administrator, "org-1", "organization", "webhook", translation).map((section) => section.id), ["webhooks"]);
  assert.deepEqual(visibleSettingsSections(administrator, "", "system", "image", translation).map((section) => section.id), ["images"]);
  assert.deepEqual(visibleSettingsSections(administrator, "", "organization", "createOrganization", translation).map((section) => section.id), ["organization-management"]);
});
