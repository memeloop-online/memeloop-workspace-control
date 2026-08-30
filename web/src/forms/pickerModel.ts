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
  return items.filter((item) => [item.workspace.name, item.workspace.short_id, item.workspace.workspace_user].some((value) => value.toLocaleLowerCase().includes(normalized)));
}

export function injectionKindLabel(item: StoredInjection, translate: (key: MessageKey) => string) {
  if (item.kind === "environment_variable") return translate("environmentVariable");
  if (item.kind === "secret_file") return translate("credentialFile");
  if (item.kind === "ssh_public_key") return translate("sshPublicKey");
  return translate("configFile");
}
