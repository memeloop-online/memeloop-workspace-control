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

test("template form normalization preserves the bounded storage policy", () => {
  const draft = emptyTemplateDraft();
  draft.name = "storage";
  draft.storagePolicy = {
    temporary_storage_gib: "48",
  };
  const parsed = templateDraftFromYaml(templateDraftToYaml(draft));
  assert.deepEqual(parsed.storagePolicy, draft.storagePolicy);
});

test("new templates emit backend-compatible storage defaults", () => {
  const draft = { ...emptyTemplateDraft(), name: "defaults" };
  const yaml = templateDraftToYaml(draft);
  assert.match(yaml, /temporary_storage_gib: 10/u);
  assert.match(yaml, /allowed_node_pools:\n      - default/u);
  assert.match(yaml, /default_node_pool: default/u);
  assert.doesNotMatch(yaml, /runtime_class_name/u);
  assert.match(yaml, /egress_policy: unrestricted/u);
});

test("internet-only egress policy round-trips through template YAML", () => {
  const draft = { ...emptyTemplateDraft(), name: "internet", egressPolicy: "internet_only" as const };
  assert.equal(templateDraftFromYaml(templateDraftToYaml(draft)).egressPolicy, "internet_only");
});

test("node pool placement round-trips through template YAML", () => {
  const draft = {
    ...emptyTemplateDraft(),
    name: "pools",
    allowedNodePools: ["default", "gpu-pool"],
    defaultNodePool: "gpu-pool",
  };
  const yaml = templateDraftToYaml(draft);
  assert.match(yaml, /placement:\n    allowed_node_pools:\n      - default\n      - gpu-pool\n    default_node_pool: gpu-pool/u);
  const parsed = templateDraftFromYaml(yaml);
  assert.deepEqual(parsed.allowedNodePools, draft.allowedNodePools);
  assert.equal(parsed.defaultNodePool, draft.defaultNodePool);
});

test("placement requires an allowed default node pool with valid names", () => {
  assert.throws(
    () => templateDraftToYaml({ ...emptyTemplateDraft(), allowedNodePools: [] }),
    (error) => error instanceof TemplateDraftError && error.code === "invalid_node_pool_placement",
  );
  assert.throws(
    () => templateDraftToYaml({ ...emptyTemplateDraft(), allowedNodePools: ["default"], defaultNodePool: "gpu-pool" }),
    (error) => error instanceof TemplateDraftError && error.code === "invalid_node_pool_placement",
  );
  assert.throws(
    () => templateDraftToYaml({ ...emptyTemplateDraft(), allowedNodePools: ["Invalid_Pool"], defaultNodePool: "Invalid_Pool" }),
    (error) => error instanceof TemplateDraftError && error.code === "invalid_node_pool_placement",
  );
});

test("duplicate node pools are normalized before writing YAML", () => {
  const yaml = templateDraftToYaml({ ...emptyTemplateDraft(), name: "dedup", allowedNodePools: ["default", "default"] });
  assert.deepEqual(templateDraftFromYaml(yaml).allowedNodePools, ["default"]);
});

test("browser desktop settings round-trip through template YAML and remain optional", () => {
  const draft = {
    ...emptyTemplateDraft(),
    name: "desktop",
    desktopEnabled: true,
    desktopPort: "6080",
    desktopDisplayName: "Security desktop",
  };
  const yaml = templateDraftToYaml(draft);
  assert.match(yaml, /desktop:\n    internal_port: 6080\n    display_name: Security desktop/u);
  assert.deepEqual(templateDraftFromYaml(yaml), draft);
  assert.doesNotMatch(templateDraftToYaml({ ...draft, desktopEnabled: false }), /desktop:/u);
});

test("browser desktop YAML rejects unavailable port values", () => {
  const yaml = templateDraftToYaml({ ...emptyTemplateDraft(), name: "desktop" }).replace(
    "  egress_policy: unrestricted\n",
    "  desktop:\n    internal_port: 0\n  egress_policy: unrestricted\n",
  );
  assert.throws(() => templateDraftFromYaml(yaml), /desktop\.internal_port/u);
  assert.throws(
    () => templateDraftToYaml({ ...emptyTemplateDraft(), desktopEnabled: true, desktopPort: "8080" }),
    TemplateDraftError,
  );
});

test("an optional RuntimeClass name round-trips through the template YAML", () => {
  const draft = { ...emptyTemplateDraft(), name: "sandbox", runtimeClassName: "gvisor-sandbox" };
  const yaml = templateDraftToYaml(draft);
  assert.match(yaml, /runtime_class_name: gvisor-sandbox/u);
  assert.equal(templateDraftFromYaml(yaml).runtimeClassName, "gvisor-sandbox");
});

test("temporary storage drafts reject empty and out-of-range values", () => {
  const emptyStorage = emptyTemplateDraft();
  emptyStorage.storagePolicy.temporary_storage_gib = "";
  assert.throws(
    () => templateDraftToYaml(emptyStorage),
    (error) => error instanceof TemplateDraftError && error.code === "invalid_template_number",
  );

  const belowMinimum = emptyTemplateDraft();
  belowMinimum.storagePolicy.temporary_storage_gib = "0";
  assert.throws(() => templateDraftToYaml(belowMinimum), TemplateDraftError);

  const aboveMaximum = emptyTemplateDraft();
  aboveMaximum.storagePolicy.temporary_storage_gib = "2049";
  assert.throws(() => templateDraftToYaml(aboveMaximum), TemplateDraftError);
});

test("template form rejects unknown fields at every schema object level", () => {
  const yaml = templateDraftToYaml({ ...emptyTemplateDraft(), name: "strict" });
  const cases = [
    yaml.replace("spec:\n", "arbitrary_typo: true\nspec:\n"),
    yaml.replace("  name: strict\n", "  name: strict\n  arbitrary_typo: true\n"),
    yaml.replace("  image: \"\"\n", "  arbitrary_typo: true\n  image: \"\"\n"),
    yaml.replace("    cpu_millis: 2000\n", "    arbitrary_typo: true\n    cpu_millis: 2000\n"),
    yaml.replace("    cpu_millis: 500\n", "    arbitrary_typo: true\n    cpu_millis: 500\n"),
    yaml.replace("    temporary_storage_gib: 10\n", "    arbitrary_typo: true\n    temporary_storage_gib: 10\n"),
  ];

  for (const candidate of cases) {
    assert.throws(() => templateDraftFromYaml(candidate), /unknown field .*arbitrary_typo/u);
  }
});
