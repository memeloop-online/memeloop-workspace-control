import type { InjectionDraft, InjectionKind, StoredInjection } from "../types";

export interface InjectionEditorDraft extends Omit<InjectionDraft, "file_mode"> {
  fileMode: string;
  storedValueAvailable: boolean;
}

/**
 * Compare the fields that can change when an injection is saved.
 *
 * `version` is assigned by the API and `storedValueAvailable` only describes
 * whether the read-only value request has completed, so neither belongs in a
 * dirty check. Labels are sorted to make their object insertion order
 * irrelevant, and valid file modes are compared by their numeric value so
 * equivalent octal spellings (644 and 0644) do not count as a change.
 */
export function injectionDraftsEqual(
  left: InjectionEditorDraft,
  right: InjectionEditorDraft,
): boolean {
  return JSON.stringify(comparableDraft(left)) === JSON.stringify(comparableDraft(right));
}

export function hasSubstantiveInjectionChanges(
  draft: InjectionEditorDraft,
  baseline: InjectionEditorDraft,
): boolean {
  return !injectionDraftsEqual(draft, baseline);
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
    storedValueAvailable: false,
    owner: null,
    group: null,
    template_selector: templateSelector,
    labels: {},
  };
}

export function draftFromStored(item: StoredInjection): InjectionEditorDraft {
  return {
    ...emptyInjectionDraft(item.template_selector),
    key: item.key,
    kind: item.kind,
    target: item.target,
    value: item.value ? { ...item.value } : { encoding: "utf8", value: "" },
    sensitive: item.sensitive,
    locked: item.locked,
    version: item.version,
    fileMode: item.file_mode === null ? "" : item.file_mode.toString(8).padStart(3, "0"),
    storedValueAvailable: item.value !== null && item.value !== undefined,
    owner: item.owner,
    group: item.group,
    labels: { ...item.labels },
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
  const { fileMode: _fileMode, storedValueAvailable: _storedValueAvailable, ...item } = draft;
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

function comparableDraft(draft: InjectionEditorDraft) {
  let fileMode: number | string | null;
  if (draft.kind === "environment_variable") {
    fileMode = null;
  } else {
    try {
      fileMode = parseFileMode(draft.kind, draft.fileMode);
    } catch {
      // Keep invalid input distinct while the user is editing it. The form
      // will report the validation error when they attempt to save.
      fileMode = draft.fileMode;
    }
  }
  return {
    key: draft.key,
    kind: draft.kind,
    target: draft.target,
    value: draft.value,
    sensitive: draft.sensitive,
    locked: draft.locked,
    fileMode,
    owner: draft.owner,
    group: draft.group,
    template_selector: draft.template_selector,
    labels: Object.fromEntries(Object.entries(draft.labels).sort(([a], [b]) => a.localeCompare(b))),
  };
}

function sshTarget(key: string) {
  const safe = key.trim().replace(/[^A-Za-z0-9._-]+/g, "-") || "injected-key";
  return `/workspace/.mwc/${safe}.pub`;
}
