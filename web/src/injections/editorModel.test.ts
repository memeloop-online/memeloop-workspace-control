import assert from "node:assert/strict";
import test from "node:test";

import {
  changeInjectionKind,
  draftFromStored,
  emptyInjectionDraft,
  injectionDraftForSave,
  parseFileMode,
} from "./editorModel.ts";

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
