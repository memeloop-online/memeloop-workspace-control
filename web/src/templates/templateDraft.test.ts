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

test("template YAML rejects removed fields and never emits them", () => {
  const yaml = templateDraftToYaml({ ...emptyTemplateDraft(), name: "historical" });
  assert.doesNotMatch(yaml, /(?:environment|preserve_home_ownership|preserve_home_root):/u);
  for (const removed of [
    "environment:\n    SHOULD_NOT_SURVIVE: value",
    "preserve_home_ownership: true",
    "preserve_home_root: true",
  ]) {
    assert.throws(
      () => templateDraftFromYaml(yaml.replace("spec:\n", `spec:\n  ${removed}\n`)),
      /Removed WorkspaceTemplate fields/u,
    );
  }
});

test("template form normalization preserves the bounded storage policy", () => {
  const draft = emptyTemplateDraft();
  draft.name = "storage";
  draft.storagePolicy = {
    runtime_tmp_memory_mib: "640",
    build_scratch_gib: "14",
    buildkit_cache_gib: "9",
    codex_scratch_gib: "3",
    home_reserve_mib: "768",
  };
  const parsed = templateDraftFromYaml(templateDraftToYaml(draft));
  assert.deepEqual(parsed.storagePolicy, draft.storagePolicy);
});

test("new templates emit backend-compatible storage and ephemeral defaults", () => {
  const draft = { ...emptyTemplateDraft(), name: "defaults" };
  const yaml = templateDraftToYaml(draft);
  assert.match(yaml, /ephemeral_storage_mib: 2048/u);
  assert.match(yaml, /ephemeral_storage_limit_mib: 14592/u);
  assert.match(yaml, /home_reserve_mib: 1024/u);
  assert.doesNotMatch(yaml, /home_reserve_mib: null/u);
});

test("an automatic home reserve remains empty in the form and round-trips as null", () => {
  const draft = { ...emptyTemplateDraft(), name: "automatic-reserve" };
  const historicalYaml = templateDraftToYaml(draft).replace("home_reserve_mib: 1024", "home_reserve_mib: null");
  const parsed = templateDraftFromYaml(historicalYaml);
  assert.equal(parsed.storagePolicy.home_reserve_mib, "");
  assert.match(templateDraftToYaml(parsed), /home_reserve_mib: null/u);
});

test("storage policy drafts reject empty required fields and reserves over ten percent", () => {
  const emptyRuntime = emptyTemplateDraft();
  emptyRuntime.storagePolicy.runtime_tmp_memory_mib = "";
  assert.throws(
    () => templateDraftToYaml(emptyRuntime),
    (error) => error instanceof TemplateDraftError && error.code === "invalid_template_number",
  );

  const excessiveReserve = emptyTemplateDraft();
  excessiveReserve.disk = "5";
  excessiveReserve.storagePolicy.home_reserve_mib = "1024";
  assert.throws(
    () => templateDraftToYaml(excessiveReserve),
    (error) => error instanceof TemplateDraftError && error.code === "invalid_template_number",
  );
});

test("explicit storage-policy boundaries match the backend contract", () => {
  const minimum = { ...emptyTemplateDraft(), name: "minimum-storage", disk: "1" };
  minimum.storagePolicy.runtime_tmp_memory_mib = "64";
  minimum.storagePolicy.home_reserve_mib = "64";
  assert.doesNotThrow(() => templateDraftToYaml(minimum));

  const maximum = { ...emptyTemplateDraft(), name: "maximum-reserve", disk: "40" };
  maximum.storagePolicy.home_reserve_mib = "4096";
  assert.doesNotThrow(() => templateDraftToYaml(maximum));

  const belowMinimum = emptyTemplateDraft();
  belowMinimum.storagePolicy.home_reserve_mib = "63";
  assert.throws(() => templateDraftToYaml(belowMinimum), TemplateDraftError);
});
