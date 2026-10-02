import type { ApiKeyScope, Principal } from "./types";

export function hasApiKeyScope(principal: Principal, scope: ApiKeyScope): boolean {
  return principal.api_key_scopes.includes(scope);
}

export function canManageSystem(principal: Principal): boolean {
  return principal.system_admin && hasApiKeyScope(principal, "manage_system");
}

export function canManageOtherUserApiKeys(principal: Principal): boolean {
  return canManageSystem(principal) && hasApiKeyScope(principal, "manage_api_keys") && principal.allowed_template_ids === null;
}

export function canManageOrganization(principal: Principal, organizationId: string, scope: "manage_organization" | "manage_members"): boolean {
  const hasAdministrativeRole = principal.system_admin || principal.memberships.some((membership) =>
    membership.organization_id === organizationId && membership.role === "organization_admin");
  return hasAdministrativeRole && hasApiKeyScope(principal, scope);
}
