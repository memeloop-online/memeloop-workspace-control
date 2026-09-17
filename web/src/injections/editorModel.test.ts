import assert from "node:assert/strict";
import test from "node:test";

import type { StoredInjection } from "../types.ts";
import {
  changeInjectionKind,
  draftFromStored,
  emptyInjectionDraft,
  injectionDraftForSave,
  parseFileMode,
} from "./editorModel.ts";

function storedInjection(overrides: Partial<StoredInjection> = {}): StoredInjection {
  return {
    key: "npmrc",
    kind: "config_file",
    target: "/workspace/.npmrc",
    scope: "workspace",
    scope_id: "ws-1",
    sensitive: false,
    locked: false,
    version: 3,
    file_mode: 0o644,
    owner: null,
    group: null,
    template_selector: null,
    labels: {},
    updated_at: 1,
    value: { encoding: "utf8", value: "registry=https://example.invalid" },
    ...overrides,
  };
}

test("file modes are parsed only when the complete value is valid octal", () => {
  assert.equal(parseFileMode("config_file", "644"), 0o644);
  assert.equal(parseFileMode("secret_file", "0600"), 0o600);
  assert.equal(parseFileMode("config_file", ""), null);
  assert.throws(() => parseFileMode("config_file", "678"), /invalid_file_mode/u);
  assert.throws(() => parseFileMode("config_file", "644junk"), /invalid_file_mode/u);
});

test("sensitive files default to mode 0600", () => {
  const draft = changeInjectionKind(emptyInjectionDraft(), "secret_file");
  assert.equal(draft.fileMode, "600");
  assert.equal(draft.sensitive, true);
  assert.equal(injectionDraftForSave(draft).file_mode, 0o600);
});

test("a fixed template selector cannot be changed by form state", () => {
  const draft = { ...emptyInjectionDraft("old-template"), key: "settings" };
  assert.equal(
    injectionDraftForSave(draft, "fixed-template").template_selector,
    "fixed-template",
  );
});

test("non-sensitive stored values load into the editor", () => {
  const draft = draftFromStored(storedInjection());
  assert.equal(draft.value.value, "registry=https://example.invalid");
  assert.equal(draft.value.encoding, "utf8");
  assert.equal(draft.storedValueAvailable, true);
});

test("sensitive values stay blank so saving replaces them", () => {
  const draft = draftFromStored(storedInjection({ sensitive: true, value: null }));
  assert.equal(draft.value.value, "");
  assert.equal(draft.sensitive, true);
  assert.equal(draft.storedValueAvailable, false);
});
