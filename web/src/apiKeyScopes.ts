import type { MessageKey } from "./i18n";
import type { ApiKeyScope } from "./types";

export const API_KEY_SCOPES = [
  { scope: "manage_api_keys", label: "scope_manage_api_keys" },
  { scope: "manage_system", label: "scope_manage_system" },
  { scope: "manage_organization", label: "scope_manage_organization" },
  { scope: "manage_members", label: "scope_manage_members" },
  { scope: "manage_locked_injections", label: "scope_manage_locked_injections" },
  { scope: "create_workspace", label: "scope_create_workspace" },
  { scope: "read_workspace", label: "scope_read_workspace" },
  { scope: "connect_workspace", label: "scope_connect_workspace" },
  { scope: "change_workspace_state", label: "scope_change_workspace_state" },
  { scope: "delete_workspace", label: "scope_delete_workspace" },
] as const satisfies ReadonlyArray<{ scope: ApiKeyScope; label: MessageKey }>;

export const API_KEY_SCOPE_LABELS: Record<string, MessageKey> = {
  "*": "scope_wildcard",
  ...Object.fromEntries(API_KEY_SCOPES.map(({ scope, label }) => [scope, label])),
};

export const DEFAULT_USER_SCOPES: ApiKeyScope[] = [
  "manage_api_keys",
  "create_workspace",
  "read_workspace",
  "connect_workspace",
  "change_workspace_state",
];

export const SYSTEM_ADMIN_SCOPES: ApiKeyScope[] = API_KEY_SCOPES.map(({ scope }) => scope);
