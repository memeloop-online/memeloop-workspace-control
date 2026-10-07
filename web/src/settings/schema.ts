import type { MessageKey } from "../i18n";
import type { Principal } from "../types";
import { canManageOrganization, canManageOtherUserApiKeys, canManageSystem, hasApiKeyScope } from "../permissions.ts";
import { settingsFields } from "./fields.ts";

export type SettingsCategory = "personal" | "organization" | "system";
export type SettingsSectionId = "profile" | "appearance" | "organization" | "organization-management" | "api-keys" | "quota" | "workspace-state" | "users" | "templates" | "webhooks" | "images" | "scaling";

export interface SettingsSection {
  id: SettingsSectionId;
  category: SettingsCategory;
  title: MessageKey;
  description?: MessageKey;
  keywords: readonly string[];
  available: (principal: Principal, organizationId: string) => boolean;
}

const anyUser = () => true;
const manageOrganization = (principal: Principal, organizationId: string) => Boolean(organizationId) && canManageOrganization(principal, organizationId, "manage_organization");
const manageMembers = (principal: Principal, organizationId: string) => Boolean(organizationId) && canManageOrganization(principal, organizationId, "manage_members");
const systemAdmin = (principal: Principal) => canManageSystem(principal);

export const settingsSections: readonly SettingsSection[] = [
  { id: "profile", category: "personal", title: "profileSettings", keywords: ["display_name", "avatar_url", "avatar", "name", "displayName"], available: anyUser },
  { id: "appearance", category: "personal", title: "appearanceSettings", keywords: ["theme", "locale", "language", "light", "dark"], available: anyUser },
  { id: "organization", category: "organization", title: "organizationSettings", description: "organizationSwitchHelp", keywords: ["organization_id", "switch", "select"], available: anyUser },
  { id: "organization-management", category: "organization", title: "organizationManagement", keywords: ["organization", "createOrganization", "deleteOrganization", "rename", "organizationName"], available: (principal, organizationId) => systemAdmin(principal) || manageOrganization(principal, organizationId) },
  { id: "api-keys", category: "personal", title: "userApiCredentials", description: "apiKeysHelp", keywords: ["api key", "token", "credentials", "copy", "revoke", "userApiCredentials", "revokeApiKey"], available: (principal) => hasApiKeyScope(principal, "manage_api_keys") },
  { id: "quota", category: "organization", title: "identityQuota", keywords: ["cpu_millis", "memory_mib", "gpu_count", "disk_gib", "editQuota", "cpu", "memory", "gpu", "disk"], available: (principal, organizationId) => Boolean(organizationId) && (manageOrganization(principal, organizationId) || manageMembers(principal, organizationId) || systemAdmin(principal)) },
  { id: "workspace-state", category: "organization", title: "workspaceState", keywords: ["workspace", "stateCount", "summary"], available: (principal, organizationId) => Boolean(organizationId) && hasApiKeyScope(principal, "read_workspace") && (manageOrganization(principal, organizationId) || manageMembers(principal, organizationId) || systemAdmin(principal)) },
  { id: "users", category: "organization", title: "usersRoles", keywords: ["user", "role", "member", "credential", "quota", "createUser", "userApiCredentials"], available: (principal, organizationId) => Boolean(organizationId) && (manageMembers(principal, organizationId) || canManageOtherUserApiKeys(principal)) },
  { id: "templates", category: "organization", title: "templates", keywords: ["template", "workspace", "cluster"], available: manageOrganization },
  { id: "webhooks", category: "organization", title: "webhook", keywords: ["webhook", "url", "secret", "event_prefix"], available: manageOrganization },
  { id: "images", category: "system", title: "imageAllowlist", keywords: ["oci", "image", "allowlist", "enabled"], available: systemAdmin },
  { id: "scaling", category: "system", title: "scaling", keywords: ["database", "replicas", "jobs", "schema_version"], available: systemAdmin },
];

export function visibleSettingsSections(principal: Principal, organizationId: string, category: SettingsCategory | "all", query: string, t: (key: MessageKey) => string): SettingsSection[] {
  const terms = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  return settingsSections.filter((section) => section.available(principal, organizationId) && (category === "all" || section.category === category) && terms.every((term) => [section.id, t(section.title), section.description ? t(section.description) : "", ...section.keywords, ...section.keywords.filter((word) => word in keywordLabels).map((word) => t(keywordLabels[word as keyof typeof keywordLabels])), ...settingsFields.filter((field) => field.section === section.id).flatMap((field) => [field.key, t(field.label), field.description ? t(field.description) : "", ...(field.options ?? []).flatMap((option) => [option.value, t(option.label)])])].join(" ").toLocaleLowerCase().includes(term)));
}

const keywordLabels = { organizationName: "organizationSettings", createOrganization: "createOrganization", deleteOrganization: "deleteOrganization", editQuota: "editQuota", stateCount: "stateCount", createUser: "createUser", userApiCredentials: "userApiCredentials", revokeApiKey: "revokeApiKey", avatar: "avatarUrl", displayName: "displayName", theme: "appearanceSettings", language: "language", copy: "copy", cpu: "cpu", memory: "memory", gpu: "gpu", disk: "disk" } as const;
