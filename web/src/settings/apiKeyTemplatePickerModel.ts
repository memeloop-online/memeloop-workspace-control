import type { WorkspaceTemplate } from "../types";

/**
 * A key may only grant templates that its authenticating principal can use.
 * A null principal allowlist means that the principal is unrestricted.
 */
export function templatesAllowedForPrincipal(
  templates: readonly WorkspaceTemplate[],
  principalAllowedTemplateIds: readonly string[] | null,
): WorkspaceTemplate[] {
  if (principalAllowedTemplateIds === null) return [...templates];
  const allowed = new Set(principalAllowedTemplateIds);
  return templates.filter((template) => allowed.has(template.id));
}

/** Drop selections that are no longer present after an org or permission change. */
export function normalizeTemplateSelection(
  selected: readonly string[],
  templates: readonly WorkspaceTemplate[],
): string[] {
  const available = new Set(templates.map((template) => template.id));
  return selected.filter((id, index) => available.has(id) && selected.indexOf(id) === index);
}

export function toggleTemplateSelection(selected: readonly string[], id: string): string[] {
  return selected.includes(id)
    ? selected.filter((item) => item !== id)
    : [...selected, id];
}

/** Keep template identity useful without exposing the full UUID in the UI. */
export function shortTemplateId(id: string): string {
  return id.slice(0, 8);
}
