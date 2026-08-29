import assert from "node:assert/strict";
import test from "node:test";

import {
  TemplateDraftError,
  emptyTemplateDraft,
  templateDraftFromYaml,
  templateDraftToYaml,
} from "./templateDraft.ts";

test("numeric fields keep an empty editing state instead of coercing it to zero", () => {
  const draft = { ...emptyTemplateDraft(), requestCpu: "" };
  assert.throws(
    () => templateDraftToYaml(draft),
    (error) => error instanceof TemplateDraftError && error.code === "invalid_template_number",
  );
});

test("resource requests cannot exceed their limits", () => {
  const draft = { ...emptyTemplateDraft(), cpu: "1000", requestCpu: "1100" };
  assert.throws(
    () => templateDraftToYaml(draft),
    (error) => error instanceof TemplateDraftError && error.code === "resource_request_exceeds_limit",
  );
});

test("numeric policies reject values that do not align to their domain step", () => {
  assert.throws(
    () => templateDraftToYaml({ ...emptyTemplateDraft(), memory: "4100" }),
    (error) => error instanceof TemplateDraftError && error.code === "invalid_template_number",
  );
});

test("template YAML normalization removes the deprecated editable environment map", () => {
  const yaml = templateDraftToYaml({ ...emptyTemplateDraft(), name: "historical" }).replace(
    "spec:\n",
    "spec:\n  environment:\n    SHOULD_NOT_SURVIVE: value\n",
  );
  const normalized = templateDraftToYaml(templateDraftFromYaml(yaml));
  assert.doesNotMatch(normalized, /SHOULD_NOT_SURVIVE/u);
  assert.doesNotMatch(normalized, /^  environment:/mu);
});
