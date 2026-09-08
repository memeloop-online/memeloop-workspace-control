import assert from "node:assert/strict";
import test from "node:test";

import type { WorkspaceTemplate } from "../types.ts";
import {
  normalizeTemplateSelection,
  shortTemplateId,
  templatesAllowedForPrincipal,
  toggleTemplateSelection,
} from "./apiKeyTemplatePickerModel.ts";

const first = { id: "11111111-1111-1111-1111-111111111111", name: "Docs" } as WorkspaceTemplate;
const second = { id: "22222222-2222-2222-2222-222222222222", name: "Games" } as WorkspaceTemplate;
const templates = [first, second];

test("an unrestricted principal can grant any loaded template", () => {
  assert.deepEqual(templatesAllowedForPrincipal(templates, null), templates);
});

test("a restricted principal can only see its template subset", () => {
  assert.deepEqual(templatesAllowedForPrincipal(templates, [second.id, "outside-org"]), [second]);
});

test("template selection drops stale and duplicate ids", () => {
  assert.deepEqual(normalizeTemplateSelection([first.id, "missing", first.id], [first]), [first.id]);
  assert.deepEqual(toggleTemplateSelection([first.id], first.id), []);
  assert.deepEqual(toggleTemplateSelection([], second.id), [second.id]);
});

test("template labels expose only a short identifier", () => {
  assert.equal(shortTemplateId(first.id), "11111111");
  assert.notEqual(shortTemplateId(first.id), first.id);
});
