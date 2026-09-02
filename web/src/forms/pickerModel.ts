import type { MessageKey } from "../i18n";
import type { StoredInjection, WorkspaceResponse } from "../types";

export function filterReferenceItems(items: StoredInjection[], query: string, translate: (key: MessageKey) => string) {
  const normalized = query.trim().toLocaleLowerCase();
  if (!normalized) return items;
  return items.filter((item) => [item.key, item.target, injectionKindLabel(item, translate)].some((value) => value.toLocaleLowerCase().includes(normalized)));
}

export function filterWorkspaces(items: WorkspaceResponse[], query: string) {
  const normalized = query.trim().toLocaleLowerCase();
  if (!normalized) return items;
  return items.filter((item) => [
    item.workspace.name,
    item.workspace.short_id,
    item.workspace.workspace_user,
    `${item.workspace.name} · ${item.workspace.short_id}`,
  ].some((value) => value.toLocaleLowerCase().includes(normalized)));
}

/**
 * A blank draft is an explicit removal of the current workspace selection.
 * Other unfinished searches remain drafts and are restored on blur or Escape.
 */
export function shouldClearWorkspaceDraft(query: string) {
  return query.trim().length === 0;
}

/**
 * Keep the first representation of a workspace while combining locally loaded
 * items with a remote search response. The local list is deliberately first:
 * it is the parent panel's current, complete representation and therefore
 * remains the authoritative option when an endpoint returns the same ID.
 */
export function mergeWorkspaces(...groups: readonly WorkspaceResponse[][]) {
  const seen = new Set<string>();
  const merged: WorkspaceResponse[] = [];
  for (const group of groups) {
    for (const item of group) {
      const id = item.workspace.id;
      if (seen.has(id)) continue;
      seen.add(id);
      merged.push(item);
    }
  }
  return merged;
}

export function injectionKindLabel(item: StoredInjection, translate: (key: MessageKey) => string) {
  if (item.kind === "environment_variable") return translate("environmentVariable");
  if (item.kind === "secret_file") return translate("credentialFile");
  if (item.kind === "ssh_public_key") return translate("sshPublicKey");
  return translate("configFile");
}
