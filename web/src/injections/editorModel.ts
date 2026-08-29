import type { InjectionDraft, InjectionKind, StoredInjection } from "../types";

export interface InjectionEditorDraft extends Omit<InjectionDraft, "file_mode"> {
  fileMode: string;
}

export const FILE_MODE_PATTERN = "(?:0)?[0-7]{3}";

export function emptyInjectionDraft(templateSelector: string | null = null): InjectionEditorDraft {
  return {
    key: "",
    kind: "config_file",
    target: "/workspace/config.yaml",
    value: { encoding: "utf8", value: "" },
    sensitive: false,
    locked: false,
    version: 0,
    fileMode: "644",
    owner: null,
    group: null,
    template_selector: templateSelector,
    labels: {},
  };
}

export function draftFromStored(item: StoredInjection): InjectionEditorDraft {
  const { template_id: legacyTemplateSelector, ...labels } = item.labels;
  return {
    ...emptyInjectionDraft(item.template_selector ?? legacyTemplateSelector ?? null),
    key: item.key,
    kind: item.kind,
    target: item.target,
    sensitive: item.sensitive,
    locked: item.locked,
    version: item.version,
    fileMode: item.file_mode === null ? "" : item.file_mode.toString(8).padStart(3, "0"),
    owner: item.owner,
    group: item.group,
    labels,
  };
}

export function changeInjectionKind(
  draft: InjectionEditorDraft,
  kind: InjectionKind,
): InjectionEditorDraft {
  const target =
    kind === "environment_variable"
      ? "EXAMPLE_VARIABLE"
      : kind === "ssh_public_key"
        ? sshTarget(draft.key)
        : kind === "secret_file"
          ? "/run/secrets/example"
          : "/workspace/config.yaml";
  return {
    ...draft,
    kind,
    target,
    sensitive: kind === "secret_file",
    fileMode:
      kind === "environment_variable" ? "" : kind === "secret_file" ? "600" : "644",
  };
}

export function changeInjectionKey(
  draft: InjectionEditorDraft,
  key: string,
): InjectionEditorDraft {
  return {
    ...draft,
    key,
    target: draft.kind === "ssh_public_key" ? sshTarget(key) : draft.target,
  };
}

export function injectionDraftForSave(
  draft: InjectionEditorDraft,
  fixedTemplateSelector?: string,
): InjectionDraft {
  const fileMode = parseFileMode(draft.kind, draft.fileMode);
  const { fileMode: _fileMode, ...item } = draft;
  return {
    ...item,
    file_mode: fileMode,
    template_selector: fixedTemplateSelector ?? draft.template_selector,
    labels: { ...draft.labels },
    value: { ...draft.value },
  };
}

export function parseFileMode(kind: InjectionKind, value: string): number | null {
  if (kind === "environment_variable") return null;
  if (value === "") return null;
  if (!new RegExp(`^${FILE_MODE_PATTERN}$`, "u").test(value)) {
    throw new Error("invalid_file_mode");
  }
  return Number.parseInt(value, 8);
}

function sshTarget(key: string) {
  const safe = key.trim().replace(/[^A-Za-z0-9._-]+/g, "-") || "injected-key";
  return `/workspace/.mwc/${safe}.pub`;
}
